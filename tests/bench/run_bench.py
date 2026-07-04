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
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import threading
import time
import wave

REPO = os.path.dirname(os.path.dirname(os.path.dirname(
    os.path.abspath(__file__))))
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

# name -> settings overrides; xy is the canonical load, takens is the
# 3D python-path context point
LIVE_CONFIGS = [
    ("gl-max-384k-ss2", {"renderer": "gl", "display_mode": "xy",
                         "scope_sample_rate": 384000, "gl_supersample": 2}),
    ("gl-384k-ss1", {"renderer": "gl", "display_mode": "xy",
                     "scope_sample_rate": 384000, "gl_supersample": 1}),
    ("cairo-max-384k", {"renderer": "cairo", "display_mode": "xy",
                        "scope_sample_rate": 384000,
                        "cairo_resolution": 1.0}),
    ("gl-default-96k", {"renderer": "gl", "display_mode": "xy",
                        "scope_sample_rate": 96000, "gl_supersample": 1}),
    ("cairo-default-96k", {"renderer": "cairo", "display_mode": "xy",
                           "scope_sample_rate": 96000,
                           "cairo_resolution": 1.0}),
    ("gl-takens-96k-ss2", {"renderer": "gl",
                           "display_mode": "xyz_takens",
                           "scope_sample_rate": 96000,
                           "gl_supersample": 2}),
]

OFFLINE_RATES = (96000, 384000)


def run(arguments, **kwargs):
    return subprocess.run(arguments, capture_output=True, text=True,
                          **kwargs)


def write_tone(path, seconds):
    """Stereo sweep + detuned right (the parity test's awkward signal),
    frequency cycling every 8 s so the XY figure stays wide forever."""
    rate = 48000
    with wave.open(path, "w") as wav_file:
        wav_file.setnchannels(2)
        wav_file.setsampwidth(2)
        wav_file.setframerate(rate)
        chunk = []
        for i in range(seconds * rate):
            t = i / rate
            frequency = 220.0 + 400.0 * ((t % 8.0) / 8.0)
            left = 0.6 * math.sin(2 * math.pi * frequency * t)
            right = 0.6 * math.sin(2 * math.pi * frequency * 1.5 * t + 0.7)
            chunk.append(struct.pack("<hh", int(left * 32767),
                                     int(right * 32767)))
            if len(chunk) >= rate:
                wav_file.writeframes(b"".join(chunk))
                chunk = []
        wav_file.writeframes(b"".join(chunk))


def gpu_busy_path():
    for path in sorted(glob.glob(
            "/sys/class/drm/card*/device/gpu_busy_percent")):
        try:
            int(open(path).read().strip())
            return path
        except (OSError, ValueError):
            continue
    return None


class GpuSampler(threading.Thread):
    def __init__(self, path):
        super().__init__(daemon=True)
        self.path = path
        self.samples = []
        self.stop_flag = threading.Event()

    def run(self):
        while not self.stop_flag.wait(0.5):
            try:
                self.samples.append(int(open(self.path).read().strip()))
            except (OSError, ValueError):
                pass

    def finish(self, skip_first):
        self.stop_flag.set()
        self.join(timeout=2)
        kept = self.samples[skip_first:]
        if not kept:
            return None
        return {"mean": round(sum(kept) / len(kept), 1), "max": max(kept)}


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


def live_run(name, overrides, tone_path, busy_path):
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
    if stderr_tail and result.get("failure"):
        result["stderr"] = stderr_tail
    status = ("FAILED: " + str(result.get("failure"))
              if result.get("failure")
              else f"{result.get('fps_median')} fps median, "
                   f"{result.get('ms_py_median')} ms py")
    print(f"  {name}: {status}", flush=True)
    return result


def offline_run(rate, tone_path):
    home = make_home(f"offline-{rate}", {
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
    print(f"  offline {rate}: "
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
    tone_path = os.path.join(SCRATCH, "tone.wav")
    offline_tone = os.path.join(SCRATCH, "tone-offline.wav")
    if not os.path.exists(tone_path):
        print("generating tones…", flush=True)
        write_tone(tone_path, TONE_SECONDS)
        write_tone(offline_tone, OFFLINE_TONE_SECONDS)

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
        for name, overrides in LIVE_CONFIGS:
            if ONLY_PREFIX and not name.startswith(ONLY_PREFIX):
                continue
            results["live"][name] = live_run(name, overrides, tone_path,
                                             busy_path)
            time.sleep(2)
        if not SKIP_OFFLINE:
            print("offline runs:", flush=True)
            for rate in OFFLINE_RATES:
                results["offline"][str(rate)] = offline_run(rate,
                                                            offline_tone)
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
