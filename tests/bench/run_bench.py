# SPDX-License-Identifier: GPL-3.0-or-later
"""v3 baseline bench orchestrator (V4PLAN wave 1, step 1).

Creates a silent null sink (phosbench), plays a deterministic tone into
it, and runs bench_probe.py once per configuration with a scratch HOME
whose settings.json pins that configuration. Also times the offline
`phosphor --render` pipeline. Everything lands in one JSON:

    tests/bench/results/v3-baseline.json

Run from the repo root on the real desktop session (needs DISPLAY):

    python3 tests/bench/run_bench.py

The tone is silent by construction (null sink), the scratch HOMEs keep
the real ~/.config/phosphor untouched, and the null sink module is
unloaded on every exit path.
"""

import glob
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(
    os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO, "tests", "bench"))

import signals
SCRATCH = os.environ.get("BENCH_SCRATCH", "/tmp/phosphor-bench")
RESULTS_PATH = os.path.join(REPO, "tests", "bench", "results",
                            "v3-baseline.json")
SINK_NAME = "phosbench"
TONE_SECONDS = 240
OFFLINE_TONE_SECONDS = 15
# env knobs for smoke tests / partial reruns
WARMUP_SECONDS = float(os.environ.get("BENCH_WARMUP_SECONDS", 6))
MEASURE_SECONDS = float(os.environ.get("BENCH_MEASURE_SECONDS", 30))
ONLY_PREFIX = os.environ.get("BENCH_ONLY", "")
SKIP_OFFLINE = bool(os.environ.get("BENCH_SKIP_OFFLINE"))
SKIP_EXISTING = bool(os.environ.get("BENCH_SKIP_EXISTING"))

BASE_SETTINGS = {
    "window_width": 1600, "window_height": 1000,
    "gain": 1.0, "auto_gain": False, "persistence": 0.7,
    "beam_energy": 8.0, "beam_focus": 1.6,
    "precompute_enabled": False, "pinned": False,
    "show_now_playing": False, "vacuum_enabled": False,
    "kit_enabled": False, "kit_path": None,
    "theme_name": "P7 Green", "grid_enabled": True,
    "amoled_background": False, "scope_glass": False,
    "ui_style": "dark", "show_pin_button": False,
    "show_fps": True, "max_fps": 0,
    "target_id": f"device:{SINK_NAME}.monitor",
}

GL_MAX = {"renderer": "gl", "display_mode": "xy",
          "scope_sample_rate": 384000, "gl_supersample": 2}
CAIRO_MAX = {"renderer": "cairo", "display_mode": "xy",
             "scope_sample_rate": 384000, "cairo_resolution": 1.0}

# Real scope-music workloads (Ben's WAV masters; the cut wavs live in
# scratch, never the repo): name -> (source, start_s, seconds, out_rate,
# loops). tp192 is 39 s so it tiles; Attack Vector is 96k/24 — at 384k
# detail the capture pipe is exactly 96k, so it flows resample-free.
MUSIC_DIRECTORY = os.path.expanduser("~/Music/WAV versions")
FILE_SIGNALS = {
    "music1": (os.path.join(MUSIC_DIRECTORY, "Attack Vector.wav"),
               60, 90, 96000, 1),
    "tp192": (os.path.join(MUSIC_DIRECTORY, "192k Test Pattern.wav"),
              0, 115, 96000, 3),
}

# (name, signal, settings overrides[, extra probe env]); frame cap is
# off in BASE_SETTINGS. The sweep entries keep their original
# (v3-baseline) names; the stress signals are the real workloads —
# dense FM chaos, full-deflection noise (fill-rate worst case), the
# studio stress-knot scene, and real scope music. The -novsync variants
# add Mesa's vblank_mode=0: with the fullscreen window unredirected the
# GTK paint clock may free-run past 165 Hz — measured, not assumed.
NOVSYNC = {"vblank_mode": "0"}
LIVE_CONFIGS = [
    ("gl-max-384k-ss2", "sweep", dict(GL_MAX)),
    ("gl-384k-ss1", "sweep", dict(GL_MAX, gl_supersample=1)),
    ("cairo-max-384k", "sweep", dict(CAIRO_MAX)),
    ("gl-default-96k", "sweep",
     dict(GL_MAX, scope_sample_rate=96000, gl_supersample=1)),
    ("cairo-default-96k", "sweep",
     dict(CAIRO_MAX, scope_sample_rate=96000)),
    ("gl-takens-96k-ss2", "sweep",
     dict(GL_MAX, display_mode="xyz_takens", scope_sample_rate=96000)),
    ("gl-max-384k-ss2--chaos", "chaos", dict(GL_MAX)),
    ("gl-max-384k-ss2--noise", "noise", dict(GL_MAX)),
    ("gl-max-384k-ss2--scene", "scene", dict(GL_MAX)),
    ("gl-384k-ss1--noise", "noise", dict(GL_MAX, gl_supersample=1)),
    ("cairo-max-384k--chaos", "chaos", dict(CAIRO_MAX)),
    ("cairo-max-384k--noise", "noise", dict(CAIRO_MAX)),
    ("cairo-max-384k--scene", "scene", dict(CAIRO_MAX)),
    ("gl-max-384k-ss2--music1", "music1", dict(GL_MAX)),
    ("gl-max-384k-ss2--tp192", "tp192", dict(GL_MAX)),
    ("cairo-max-384k--music1", "music1", dict(CAIRO_MAX)),
    ("gl-max-384k-ss2--noise-novsync", "noise", dict(GL_MAX), NOVSYNC),
    ("gl-max-384k-ss2--music1-novsync", "music1", dict(GL_MAX), NOVSYNC),
    ("gl-default-96k--novsync", "sweep",
     dict(GL_MAX, scope_sample_rate=96000, gl_supersample=1), NOVSYNC),
]

# (result key, signal, detail rate)
OFFLINE_RUNS = [
    ("96000", "sweep", 96000),
    ("384000", "sweep", 384000),
    ("96000--chaos", "chaos", 96000),
    ("384000--chaos", "chaos", 384000),
    ("384000--noise", "noise", 384000),
]


def run(arguments, **kwargs):
    return subprocess.run(arguments, capture_output=True, text=True,
                          **kwargs)


def gpu_busy_path():
    for path in sorted(glob.glob(
            "/sys/class/drm/card*/device/gpu_busy_percent")):
        try:
            int(open(path).read().strip())
            return path
        except (OSError, ValueError):
            continue
    return None


def _current_sclk_mhz(path):
    """The starred line of pp_dpm_sclk, e.g. '1: 2321Mhz *'."""
    for line in open(path):
        if "*" in line:
            match = re.search(r"(\d+)\s*Mhz", line, re.IGNORECASE)
            if match:
                return int(match.group(1))
    return None


class GpuSampler(threading.Thread):
    """gpu_busy_percent + current core clock at 2 Hz — busy% alone lies
    when power management moves the clock underneath it."""

    def __init__(self, busy_path):
        super().__init__(daemon=True)
        self.busy_path = busy_path
        sclk = os.path.join(os.path.dirname(busy_path), "pp_dpm_sclk")
        self.sclk_path = sclk if os.path.exists(sclk) else None
        self.samples = []
        self.sclk_samples = []
        self.stop_flag = threading.Event()

    def run(self):
        while not self.stop_flag.wait(0.5):
            try:
                self.samples.append(
                    int(open(self.busy_path).read().strip()))
            except (OSError, ValueError):
                pass
            if self.sclk_path:
                try:
                    mhz = _current_sclk_mhz(self.sclk_path)
                    if mhz is not None:
                        self.sclk_samples.append(mhz)
                except OSError:
                    pass

    def finish(self, skip_first):
        self.stop_flag.set()
        self.join(timeout=2)
        kept = self.samples[skip_first:]
        if not kept:
            return None
        result = {"mean": round(sum(kept) / len(kept), 1),
                  "max": max(kept)}
        clocks = self.sclk_samples[skip_first:]
        if clocks:
            result["sclk_mhz_mean"] = round(sum(clocks) / len(clocks))
            result["sclk_mhz_min"] = min(clocks)
            result["sclk_mhz_max"] = max(clocks)
        return result


def make_home(name, overrides):
    home = os.path.join(SCRATCH, f"home-{name}")
    shutil.rmtree(home, ignore_errors=True)
    os.makedirs(os.path.join(home, ".config", "phosphor"))
    pulse_config = os.path.expanduser("~/.config/pulse")
    if os.path.isdir(pulse_config):
        shutil.copytree(pulse_config,
                        os.path.join(home, ".config", "pulse"))
    settings = dict(BASE_SETTINGS)
    settings.update(overrides)
    with open(os.path.join(home, ".config", "phosphor",
                           "settings.json"), "w") as handle:
        json.dump(settings, handle, indent=2)
    return home


def collect_environment():
    def out(arguments):
        try:
            return run(arguments).stdout.strip()
        except OSError:
            return None
    gtk = run([sys.executable, "-c",
               "import gi; gi.require_version('Gtk','3.0'); "
               "from gi.repository import Gtk; print('%d.%d.%d' % ("
               "Gtk.get_major_version(), Gtk.get_minor_version(), "
               "Gtk.get_micro_version()))"]).stdout.strip()
    numpy_version = run([sys.executable, "-c",
                         "import numpy; print(numpy.__version__)"
                         ]).stdout.strip()
    native = run([sys.executable, "-c",
                  "import sys; sys.path.insert(0, '.'); "
                  "import phosphor_signal; "
                  "print(phosphor_signal.native_available())"],
                 cwd=REPO).stdout.strip()
    glx = out(["sh", "-c", "glxinfo -B 2>/dev/null | grep -E "
               "'OpenGL renderer|OpenGL version' | head -2"])
    return {
        "date": time.strftime("%Y-%m-%d %H:%M %z"),
        "git_head": out(["git", "-C", REPO, "rev-parse", "HEAD"]),
        "kernel": out(["uname", "-r"]),
        "cpu": out(["sh", "-c",
                    "grep -m1 'model name' /proc/cpuinfo | cut -d: -f2"]),
        "gpu": glx,
        "monitor": out(["sh", "-c",
                        "xrandr 2>/dev/null | grep -m1 '\\*'"]),
        "audio_server": out(["sh", "-c",
                             "pactl info | grep 'Server Name'"]),
        "python": sys.version.split()[0],
        "gtk": gtk,
        "numpy": numpy_version,
        "native_core_available": native,
        "warmup_seconds": WARMUP_SECONDS,
        "measure_seconds": MEASURE_SECONDS,
    }


def cut_file_signal(name):
    """Deterministic ffmpeg cut of a real music workload into scratch."""
    source, start, seconds, rate, loops = FILE_SIGNALS[name]
    path = os.path.join(SCRATCH, f"signal-{name}.wav")
    if os.path.exists(path):
        return path
    if not os.path.exists(source):
        return None
    cut = run(["ffmpeg", "-y", "-loglevel", "error",
               "-stream_loop", str(loops - 1),
               "-ss", str(start), "-i", source, "-t", str(seconds),
               "-ac", "2", "-ar", str(rate), "-c:a", "pcm_s16le", path])
    return path if cut.returncode == 0 else None


def live_run(name, signal_name, overrides, tone_path, busy_path,
             signal_hashes, extra_env=None):
    home = make_home(name, overrides)
    result_path = os.path.join(SCRATCH, f"result-{name}.json")
    environment = dict(os.environ)
    environment.update({
        "HOME": home,
        "GDK_BACKEND": "x11",
        "BENCH_RESULT": result_path,
        "BENCH_WARMUP_SECONDS": str(WARMUP_SECONDS),
        "BENCH_MEASURE_SECONDS": str(MEASURE_SECONDS),
    })
    environment.update(extra_env or {})
    player = subprocess.Popen(
        ["paplay", f"--device={SINK_NAME}", tone_path],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    sampler = GpuSampler(busy_path) if busy_path else None
    if sampler:
        sampler.start()
    try:
        probe = subprocess.run(
            [sys.executable, os.path.join("tests", "bench",
                                          "bench_probe.py")],
            cwd=REPO, env=environment, capture_output=True, text=True,
            timeout=WARMUP_SECONDS + MEASURE_SECONDS + 60)
        stderr_tail = probe.stderr.strip()[-500:]
    except subprocess.TimeoutExpired:
        stderr_tail = "orchestrator timeout"
    finally:
        player.terminate()
        player.wait(timeout=5)
    gpu = (sampler.finish(skip_first=int(2 * WARMUP_SECONDS))
           if sampler else None)
    try:
        with open(result_path) as handle:
            result = json.load(handle)
    except (OSError, ValueError):
        result = {"failure": f"no result file; stderr: {stderr_tail}"}
    result["gpu_busy_percent"] = gpu
    result["signal"] = {"name": signal_name,
                        "sha256": signal_hashes[signal_name]}
    if stderr_tail and result.get("failure"):
        result["stderr"] = stderr_tail
    status = ("FAILED: " + str(result.get("failure"))
              if result.get("failure")
              else f"{result.get('fps_median')} fps median, "
                   f"{result.get('ms_py_median')} ms py")
    print(f"  {name}: {status}", flush=True)
    return result


def offline_run(key, rate, tone_path):
    home = make_home(f"offline-{key}", {
        "renderer": "gl", "display_mode": "xy",
        "scope_sample_rate": rate})
    environment = dict(os.environ)
    environment["HOME"] = home
    output = os.path.join(SCRATCH, f"render-{rate}.mp4")
    started = time.monotonic()
    try:
        render = subprocess.run(
            [sys.executable, "phosphor.py", "--render", tone_path,
             output, "--rate", str(rate)],
            cwd=REPO, env=environment, capture_output=True, text=True,
            timeout=900)
        failed = render.returncode != 0
        tail = (render.stderr or render.stdout).strip()[-300:]
    except subprocess.TimeoutExpired:
        failed, tail = True, "timeout after 900s"
    wall = time.monotonic() - started
    frames = OFFLINE_TONE_SECONDS * 60          # EXPORT_FPS = 60
    result = {
        "rate": rate,
        "wall_seconds": round(wall, 1),
        "frames": frames,
        "fps_equivalent": round(frames / wall, 1),
        "realtime_factor": round(OFFLINE_TONE_SECONDS / wall, 2),
    }
    if failed:
        result = {"rate": rate, "failure": tail}
    print(f"  offline {key}: "
          f"{result.get('fps_equivalent', 'FAILED')} fps-equivalent",
          flush=True)
    return result


def main():
    if not os.environ.get("DISPLAY"):
        print("no DISPLAY — the live bench needs the real desktop",
              file=sys.stderr)
        return 1
    os.makedirs(SCRATCH, exist_ok=True)
    os.makedirs(os.path.dirname(RESULTS_PATH), exist_ok=True)
    print("preparing signals…", flush=True)
    live_wavs, offline_wavs, signal_hashes = {}, {}, {}
    for name in signals.SIGNAL_NAMES:
        live_wavs[name] = signals.ensure_wav(name, TONE_SECONDS, SCRATCH)
        offline_wavs[name] = signals.ensure_wav(
            name, OFFLINE_TONE_SECONDS, SCRATCH)
        signal_hashes[name] = hashlib.sha256(
            open(live_wavs[name], "rb").read()).hexdigest()
    for name in FILE_SIGNALS:
        path = cut_file_signal(name)
        if path is None:
            print(f"  {name}: source missing/cut failed — its runs "
                  f"will be skipped", file=sys.stderr)
            continue
        live_wavs[name] = path
        signal_hashes[name] = hashlib.sha256(
            open(path, "rb").read()).hexdigest()

    loaded = run(["pactl", "load-module", "module-null-sink",
                  f"sink_name={SINK_NAME}",
                  "sink_properties=device.description=PhosphorBench"])
    if loaded.returncode != 0:
        print(f"null sink failed: {loaded.stderr}", file=sys.stderr)
        return 1
    module_id = loaded.stdout.strip()
    busy_path = gpu_busy_path()

    # merge into existing results so a filtered rerun (BENCH_ONLY=…)
    # refreshes one entry instead of discarding the rest of the baseline
    results = {"env": collect_environment(), "live": {}, "offline": {}}
    try:
        with open(RESULTS_PATH) as handle:
            previous = json.load(handle)
        results["live"] = previous.get("live", {})
        results["offline"] = previous.get("offline", {})
    except (OSError, ValueError):
        pass
    try:
        print("live runs (each ≈ %ds):" % (WARMUP_SECONDS
                                           + MEASURE_SECONDS), flush=True)
        for config in LIVE_CONFIGS:
            name, signal_name, overrides = config[:3]
            extra_env = config[3] if len(config) > 3 else None
            if ONLY_PREFIX and not name.startswith(ONLY_PREFIX):
                continue
            if SKIP_EXISTING and name in results["live"]:
                continue
            if signal_name not in live_wavs:
                continue                      # missing music source
            results["live"][name] = live_run(
                name, signal_name, overrides, live_wavs[signal_name],
                busy_path, signal_hashes, extra_env)
            time.sleep(2)
        if not SKIP_OFFLINE:
            print("offline runs:", flush=True)
            for key, signal_name, rate in OFFLINE_RUNS:
                if ONLY_PREFIX and not key.startswith(ONLY_PREFIX):
                    continue
                results["offline"][key] = offline_run(
                    key, rate, offline_wavs[signal_name])
                results["offline"][key]["signal"] = signal_name
    finally:
        unload = run(["pactl", "unload-module", module_id])
        if unload.returncode != 0:
            print(f"WARNING: could not unload null sink module "
                  f"{module_id}: {unload.stderr}", file=sys.stderr)

    with open(RESULTS_PATH, "w") as handle:
        json.dump(results, handle, indent=2)
    print(f"results → {RESULTS_PATH}")
    failed = [name for name, result in results["live"].items()
              if result.get("failure")]
    if failed:
        print(f"FAILED runs: {', '.join(failed)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
