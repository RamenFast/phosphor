# SPDX-License-Identifier: GPL-3.0-or-later
"""Capture golden fixtures from the pristine v3 engine (V4PLAN wave 1.1).

    python3 tests/capture_golden.py --record

regenerates tests/golden/ from scratch: python-engine segment fixtures
(the v4 parity reference), raw kit-chain audio, .phos round-trip files,
and — when the v3 Rust core is loadable — the labeled rust-v3 secondary
set including its production sinc oversampling. Regeneration is always a
deliberate act, same as tests/test_studio_scenes.py --record; the replay
gate is tests/test_golden_replay.py.

Every case is captured twice with fresh objects and byte-compared before
anything is written — a nondeterministic case aborts the run.
"""

import json
import os
import shutil
import subprocess
import sys

TESTS_DIRECTORY = os.path.dirname(os.path.abspath(__file__))
PROJECT = os.path.dirname(TESTS_DIRECTORY)
sys.path.insert(0, TESTS_DIRECTORY)
sys.path.insert(0, PROJECT)

import golden_lib as lib


def _capture_environment(engine_label):
    import numpy
    import platform
    head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=PROJECT,
                          capture_output=True, text=True).stdout.strip()
    import datetime
    return {
        "captured_utc": datetime.datetime.now(
            datetime.timezone.utc).isoformat(timespec="seconds"),
        "git_head": head,
        "python": sys.version.split()[0],
        "numpy": numpy.__version__,
        "platform": platform.platform(),
        "engine": engine_label,
        "tolerance_contract": {
            "note": "fixtures are exact bytes from this engine; a ported "
                    "engine (v4 rust) compares against them with the v3 "
                    "parity tolerances below (tests/test_native_parity.py "
                    "precedent). The replay test on THIS engine is exact.",
            "coordinate_px": 0.05,
            "intensity": 5e-3,
            "segment_counts": "must match exactly",
        },
    }


def _write_inputs(cases_list):
    """Generate every referenced input signal once; returns {file: sha}."""
    needed = {}
    for case in cases_list:
        reference = case["input"]
        needed[reference["file"]] = reference["signal"]
    checksums = {}
    for file_name, signal in sorted(needed.items()):
        rate = int(file_name.rsplit("-", 1)[1].split(".")[0])
        data = lib.make_signal(signal, rate).tobytes()
        path = os.path.join(lib.GOLDEN, "inputs", file_name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as handle:
            handle.write(data)
        checksums[file_name] = lib.sha256(data)
    return checksums


def _input_bytes(case):
    path = os.path.join(lib.GOLDEN, "inputs", case["input"]["file"])
    with open(path, "rb") as handle:
        data = handle.read()
    needed = case["input"]["floats"] * 4
    assert len(data) >= needed, (case["name"], len(data), needed)
    return data[:needed]


def _capture_segment_cases(cases, output_directory):
    total_bytes = 0
    for case in cases:
        input_bytes = _input_bytes(case)
        counts_a, bytes_a = lib.run_case(case, input_bytes)
        counts_b, bytes_b = lib.run_case(case, input_bytes)
        if counts_a != counts_b or bytes_a != bytes_b:
            raise SystemExit(f"NONDETERMINISTIC: {case['name']}")
        case_record = dict(case)
        case_record["input"] = dict(case["input"])
        case_record["input"]["sha256"] = lib.sha256(input_bytes)
        case_record["segment_counts_recorded"] = counts_a
        case_record["segments_sha256"] = lib.sha256(bytes_a)
        base = os.path.join(output_directory, case["name"])
        with open(base + ".segments.bin", "wb") as handle:
            handle.write(bytes_a)
        lib.write_json(base + ".json", case_record)
        total_bytes += len(bytes_a)
        print(f"  {case['name']}: {sum(counts_a)} segments over "
              f"{len(counts_a)} recorded calls")
    return total_bytes


def _capture_kit_audio():
    directory = os.path.join(lib.GOLDEN, "kits")
    os.makedirs(directory, exist_ok=True)
    for case in lib.kit_audio_cases():
        input_bytes = _input_bytes(case)
        out_a = lib.run_kit_audio_case(case, input_bytes)
        out_b = lib.run_kit_audio_case(case, input_bytes)
        if out_a != out_b:
            raise SystemExit(f"NONDETERMINISTIC: {case['name']}")
        record = dict(case)
        record["kit"] = dict(case["kit"])
        record["kit"]["stages"] = [
            [op, list(params)]
            for op, params in lib.resolve_kit_stages(case)]
        record["input"] = dict(case["input"])
        record["input"]["sha256"] = lib.sha256(input_bytes)
        record["output"] = {"file": case["name"] + ".out.f32",
                            "sha256": lib.sha256(out_a),
                            "floats": len(out_a) // 4}
        with open(os.path.join(directory, record["output"]["file"]),
                  "wb") as handle:
            handle.write(out_a)
        lib.write_json(os.path.join(directory, case["name"] + ".json"),
                       record)
        print(f"  {case['name']}: {len(out_a) // 4} floats")


PHOS_FIXTURES = (
    {"name": "plain", "rate": 48000, "signal": "sine", "seconds": 0.15,
     "title": "golden postcard", "credit": "TURTLE VECTOR",
     "source": "golden-sine.flac"},
    {"name": "unicode", "rate": 96000, "signal": "chord", "seconds": 0.10,
     "title": "Türtle ⚡ \U0001f422 postcard",
     "credit": "bénédiction ✨",
     "source": "günther—mix.flac"},
    {"name": "overlong", "rate": 48000, "signal": "sweep", "seconds": 0.10,
     "title": "A" * 300, "credit": "é" * 200,
     "source": "s" * 120 + ".flac"},
)


def _capture_phos():
    """Build .phos postcards through the real v3 writer path: a cache-style
    stream (pack_header + s16le payload) restamped by export_postcard()."""
    import numpy
    import phosphor_precompute

    directory = os.path.join(lib.GOLDEN, "phos")
    os.makedirs(directory, exist_ok=True)
    for fixture in PHOS_FIXTURES:
        audio = lib.make_signal(fixture["signal"], fixture["rate"],
                                fixture["seconds"])
        # fixture-side f32 -> s16 (the v3 cache writer receives s16 straight
        # from ffmpeg; this stands in for that decode, deterministically)
        payload = numpy.round(
            numpy.clip(audio.astype(numpy.float64), -1.0, 1.0)
            * 32767.0).astype("<i2").tobytes()
        frames = len(payload) // 4
        scratch = os.path.join(directory, fixture["name"] + ".cache.tmp")
        with open(scratch, "wb") as handle:
            handle.write(phosphor_precompute.pack_header(
                {"rate": fixture["rate"], "frames": frames,
                 "source": fixture["source"]}))
            handle.write(payload)
        destination = os.path.join(directory, fixture["name"] + ".phos")
        phosphor_precompute.export_postcard(
            scratch, destination, fixture["title"], fixture["credit"])
        os.unlink(scratch)

        header = phosphor_precompute.read_header(destination)
        assert header is not None, fixture["name"]
        with open(destination, "rb") as handle:
            file_bytes = handle.read()
        assert len(file_bytes) == phosphor_precompute.HEADER_BYTES + len(payload)
        track = phosphor_precompute.PrecomputedTrack(destination)
        assert track.frame_count == frames
        track.close()
        first = numpy.frombuffer(payload[:32], dtype="<i2").astype(
            numpy.float32) / phosphor_precompute.INT16_SCALE
        lib.write_json(os.path.join(directory, fixture["name"] + ".json"), {
            "file": fixture["name"] + ".phos",
            "file_sha256": lib.sha256(file_bytes),
            "header_bytes": phosphor_precompute.HEADER_BYTES,
            "requested": {"title": fixture["title"],
                          "credit": fixture["credit"],
                          "source": fixture["source"]},
            "header_parsed": header,
            "frames": frames,
            "rate": fixture["rate"],
            "payload_sha256": lib.sha256(payload),
            "first_decoded_f32": [float(value) for value in first],
            "decode_contract": "f32 = s16le / 32767.0",
        })
        print(f"  phos {fixture['name']}: title -> "
              f"{header.get('title', '')!r}")


def record_reference():
    assert os.environ.get("PHOSPHOR_NO_NATIVE"), \
        "reference capture requires PHOSPHOR_NO_NATIVE=1"
    import phosphor_signal
    assert not phosphor_signal.native_available()
    cases = lib.reference_cases()
    inputs = _write_inputs(cases + lib.kit_audio_cases())
    print(f"inputs: {len(inputs)} files")
    directory = os.path.join(lib.GOLDEN, "cases")
    os.makedirs(directory, exist_ok=True)
    print(f"reference cases ({len(cases)}):")
    _capture_segment_cases(cases, directory)
    print("kit audio:")
    _capture_kit_audio()
    print("phos round-trips:")
    _capture_phos()
    manifest = _capture_environment("numpy (python reference)")
    manifest["inputs"] = inputs
    manifest["cases"] = [case["name"] for case in cases]
    manifest["kit_audio"] = [case["name"]
                             for case in lib.kit_audio_cases()]
    manifest["phos"] = [fixture["name"] for fixture in PHOS_FIXTURES]
    lib.write_json(os.path.join(lib.GOLDEN, "manifest.json"), manifest)
    print(f"reference set: {len(cases)} segment cases recorded")


def record_native():
    import phosphor_signal
    assert phosphor_signal.native_available(), "native core not loadable"
    cases = lib.native_cases()
    directory = os.path.join(lib.GOLDEN, "native-v3", "cases")
    os.makedirs(directory, exist_ok=True)
    print(f"native-v3 cases ({len(cases)}):")
    _capture_segment_cases(cases, directory)
    manifest = _capture_environment("rust-v3 (core/ cdylib, API v2)")
    manifest["cases"] = [case["name"] for case in cases]
    lib.write_json(os.path.join(lib.GOLDEN, "native-v3", "manifest.json"),
                   manifest)
    print(f"native-v3 set: {len(cases)} segment cases recorded")


def orchestrate():
    if os.path.isdir(lib.GOLDEN):
        shutil.rmtree(lib.GOLDEN)
    os.makedirs(lib.GOLDEN)

    reference_env = dict(os.environ)
    reference_env["PHOSPHOR_NO_NATIVE"] = "1"
    subprocess.run([sys.executable, os.path.abspath(__file__),
                    "--record-reference"], env=reference_env, check=True)

    native_env = {key: value for key, value in os.environ.items()
                  if key != "PHOSPHOR_NO_NATIVE"}
    probe = subprocess.run(
        [sys.executable, "-c",
         f"import sys; sys.path.insert(0, {PROJECT!r}); "
         "import phosphor_signal; "
         "sys.exit(0 if phosphor_signal.native_available() else 3)"],
        env=native_env)
    if probe.returncode == 0:
        subprocess.run([sys.executable, os.path.abspath(__file__),
                        "--record-native"], env=native_env, check=True)
    else:
        print("native core not loadable: skipping the rust-v3 set")

    size = subprocess.run(["du", "-sh", lib.GOLDEN], capture_output=True,
                          text=True).stdout.split()[0]
    print(f"tests/golden: {size}")


if __name__ == "__main__":
    if "--record-reference" in sys.argv:
        record_reference()
    elif "--record-native" in sys.argv:
        record_native()
    elif "--record" in sys.argv:
        orchestrate()
    else:
        print(__doc__)
        raise SystemExit(2)
