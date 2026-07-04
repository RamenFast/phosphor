# SPDX-License-Identifier: GPL-3.0-or-later
"""In-process FPS probe for the v3 baseline (V4PLAN wave 1, step 1).

Launches the real Phosphor app inside this process, waits for the live
pipeline to warm up, then samples the FPS overlay (the app's own honest
readout: "GPU·rs · 143 fps · 2.1ms py · max 12ms") once a second and
writes aggregates as JSON.

Doctrine from HANDOFF (do not relearn):
  - Gio.ApplicationFlags.NON_UNIQUE, or run() forwards to a running
    Phosphor and this probe measures nothing.
  - Run with a scratch HOME (the orchestrator prepares settings.json
    there); Ben's real settings are never touched.
  - The tone driving the capture must outlast the run.
  - The label only refreshes while Show FPS is on and frames draw; a
    stale label means the scope went quiet — that is a failed run, not
    a slow one.

Environment: BENCH_RESULT (output JSON path), BENCH_WARMUP_SECONDS,
BENCH_MEASURE_SECONDS. Exit 0 with a result file on success.
"""

import json
import os
import re
import resource
import statistics
import sys
import time

sys.path.insert(0, os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gio, GLib

import phosphor
import phosphor_signal

RESULT_PATH = os.environ.get("BENCH_RESULT", "/tmp/bench_result.json")
WARMUP_SECONDS = float(os.environ.get("BENCH_WARMUP_SECONDS", "6"))
MEASURE_SECONDS = float(os.environ.get("BENCH_MEASURE_SECONDS", "30"))
LABEL_PATTERN = re.compile(
    r"(?P<engine>\S+) · (?P<fps>\d+) fps · (?P<ms>[\d.]+)ms py · "
    r"max (?P<gap>\d+)ms")


class Probe:
    def __init__(self, application):
        self.application = application
        self.window = None
        self.samples = []          # (fps, ms_py, worst_gap_ms)
        self.engine_label = None
        self.allocation = None
        self.stale_count = 0
        self.last_window_start = None
        self.fullscreen_retries = 0
        self.cpu_start = None
        self.wall_start = None
        self.failure = None

    def begin(self):
        self.window = self.application.props.active_window
        if self.window is None:
            self.failure = "no window after activate"
            return self.finish()
        self.window.fullscreen()
        GLib.timeout_add(int(WARMUP_SECONDS * 1000), self.start_measuring)
        # hard guard: never hang the orchestrator
        GLib.timeout_add(
            int((WARMUP_SECONDS + MEASURE_SECONDS + 25) * 1000),
            self.abort_late)
        return False

    def start_measuring(self):
        allocation = self.window.display_stack.get_allocation()
        # the fullscreen request can lose a race with the WM; measuring
        # windowed would silently change the workload, so insist first
        screen = self.window.get_screen()
        if (allocation.width < screen.get_width() - 64
                and self.fullscreen_retries < 3):
            self.fullscreen_retries += 1
            self.window.fullscreen()
            GLib.timeout_add(2500, self.start_measuring)
            return False
        usage = resource.getrusage(resource.RUSAGE_SELF)
        self.cpu_start = usage.ru_utime + usage.ru_stime
        self.wall_start = time.monotonic()
        self.allocation = [allocation.width, allocation.height]
        self.engine_label = self.window.segment_computer.engine
        GLib.timeout_add(1000, self.sample)
        return False

    def sample(self):
        text = self.window.fps_label.get_text()
        match = LABEL_PATTERN.match(text)
        # liveness comes from the counter window itself, not the text:
        # identical readouts are legal at a steady vsync lock, but
        # _fps_window_start only advances while frames actually draw
        window_start = self.window._fps_window_start
        alive = window_start != self.last_window_start
        self.last_window_start = window_start
        if match is None or not alive:
            self.stale_count += 1
            if self.stale_count >= 5:
                self.failure = (f"fps label stale/unparsable for "
                                f"{self.stale_count}s: {text!r} — "
                                f"scope quiet? capture dead?")
                return self.finish()
        else:
            self.stale_count = 0
            self.samples.append((int(match.group("fps")),
                                 float(match.group("ms")),
                                 int(match.group("gap"))))
        if time.monotonic() - self.wall_start >= MEASURE_SECONDS:
            return self.finish()
        return True

    def abort_late(self):
        self.failure = self.failure or "hard timeout"
        return self.finish()

    def finish(self):
        usage = resource.getrusage(resource.RUSAGE_SELF)
        wall = (time.monotonic() - self.wall_start
                if self.wall_start else 0.0)
        result = {
            "failure": self.failure,
            "engine": self.engine_label,
            "canvas": self.allocation,
            "sample_count": len(self.samples),
            "wall_seconds": round(wall, 2),
        }
        if self.samples and not self.failure:
            fps_values = [sample[0] for sample in self.samples]
            ms_values = [sample[1] for sample in self.samples]
            gap_values = [sample[2] for sample in self.samples]
            result.update({
                "fps_median": statistics.median(fps_values),
                "fps_mean": round(statistics.fmean(fps_values), 1),
                "fps_min": min(fps_values),
                "fps_max": max(fps_values),
                "ms_py_median": statistics.median(ms_values),
                "worst_gap_ms": max(gap_values),
                "process_cpu_fraction": round(
                    (usage.ru_utime + usage.ru_stime - self.cpu_start)
                    / wall, 3) if wall else None,
                "samples": self.samples,
            })
        settings = self.window.settings if self.window else None
        if settings is not None:
            result["config"] = {
                "renderer": settings.renderer,
                "mode": settings.display_mode,
                "scope_sample_rate": settings.scope_sample_rate,
                "gl_supersample": settings.gl_supersample,
                "cairo_resolution": settings.cairo_resolution,
                "max_fps": settings.max_fps,
                "plan_feed": list(phosphor_signal.plan_feed(
                    settings.scope_sample_rate)),
            }
        with open(RESULT_PATH, "w") as result_file:
            json.dump(result, result_file, indent=2)
        self.application.quit()
        return False


def main():
    application = phosphor.PhosphorApplication()
    application.set_flags(Gio.ApplicationFlags.NON_UNIQUE)
    probe = Probe(application)
    application.connect(
        "activate", lambda app: GLib.timeout_add(500, probe.begin))
    application.run([])
    if probe.failure:
        print(f"PROBE FAILED: {probe.failure}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
