# SPDX-License-Identifier: GPL-3.0-or-later
"""Golden-file tests for phosphor-studio scene compilation.

A scene document must compile to bit-identical samples forever — the
hash pins it. If a change here is intentional (the compiler's sound
actually changed), re-record the hashes and say so in the commit.
Run directly:  python3 tests/test_studio_scenes.py
"""

import json
import os
import subprocess
import sys

PROJECT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, PROJECT)

import phosphor_studio

GOLDEN = {
    "breathing-dot.scene.json": None,   # filled by --record
    "turtle.scene.json": None,
}
GOLDEN_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                           "studio_golden.json")


def scene_hash(name):
    document = phosphor_studio.load_scene(
        os.path.join(PROJECT, "scenes", name))
    frames, rate = phosphor_studio.compile_scene(document, sample_rate=48000)
    return phosphor_studio.scene_summary(document, frames, rate)["sha256"]


def test_scenes_match_golden():
    with open(GOLDEN_PATH) as golden_file:
        golden = json.load(golden_file)
    for name, expected in golden.items():
        actual = scene_hash(name)
        assert actual == expected, (
            f"{name}: compiled hash {actual} != golden {expected} — "
            "the compiler's output changed")
        print(f"  {name}: {actual[:16]}… ✓")


def test_validation_errors_name_the_key():
    try:
        phosphor_studio.shape_points({"kind": "dodecahedron"})
    except phosphor_studio.SceneError as error:
        assert error.json_path == "$.shape.kind"
    else:
        raise AssertionError("unknown shape kind was accepted")


def test_cli_exit_codes():
    turtle = os.path.join(PROJECT, "scenes", "turtle.scene.json")
    ok = subprocess.run(
        [sys.executable, os.path.join(PROJECT, "phosphor_studio.py"),
         "--output", "json", "validate", turtle],
        capture_output=True, text=True)
    assert ok.returncode == 0 and json.loads(ok.stdout)["ok"] is True
    bad = subprocess.run(
        [sys.executable, os.path.join(PROJECT, "phosphor_studio.py"),
         "--output", "json", "validate", "/nonexistent.scene.json"],
        capture_output=True, text=True)
    assert bad.returncode == 3
    payload = json.loads(bad.stdout)
    assert payload["ok"] is False and "message" in payload["error"]
    print("  cli exit codes ✓")


if __name__ == "__main__":
    if "--record" in sys.argv:
        golden = {name: scene_hash(name) for name in GOLDEN}
        with open(GOLDEN_PATH, "w") as golden_file:
            json.dump(golden, golden_file, indent=2)
        print(f"recorded: {golden}")
    else:
        test_scenes_match_golden()
        test_validation_errors_name_the_key()
        test_cli_exit_codes()
        print("STUDIO OK")
