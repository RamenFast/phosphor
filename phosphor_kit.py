# SPDX-License-Identifier: GPL-3.0-or-later
"""Signal kits (.phoskit): shareable transform chains for the scope.

A kit is a small JSON document describing a chain of signal-space
operations applied live to whatever the listener plays — a friend sends a
manipulation into your music. Each stage is a (possibly time-varying) 2x2
matrix over the stereo frame, or a per-channel delay; chains compose in
order, upstream of every display mode, so a kit bends the XY figure, the
goniometer, the tunnel — all of it.

    {"phoskit": 1, "name": "orbit", "author": "ben",
     "stages": [{"op": "rotate", "hz": 0.08},
                {"op": "midside", "width": 1.4}]}

The same chain runs in the Python signal path and the Rust core (bit-close,
parity-tested); phosphor_signal owns the DSP, this module owns the format.
"""

import json
import os

KIT_DIRECTORY = os.path.expanduser("~/.local/share/phosphor/kits")
SYSTEM_KIT_DIRECTORY = "/usr/share/phosphor/kits"
BUNDLED_KIT_DIRECTORY = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "kits")   # running from a checkout
FORMAT_VERSION = 1
MAX_STAGES = 16

# Canonical parameter layout per op: (json_key, default, minimum, maximum).
# Order defines the packed [p0..p3] the engines consume; UI editors and the
# FFI both read this table, so a new op lands everywhere at once.
OPERATIONS = {
    "rotate": (
        ("hz", 0.05, -4.0, 4.0),        # revolutions advance at this rate
        ("angle", 0.0, -3.14159, 3.14159),  # plus a fixed offset (radians)
    ),
    "midside": (
        ("width", 1.4, 0.0, 4.0),       # stereo width: 0 mono .. 4 exploded
    ),
    "ringmod": (
        ("hz", 3.0, 0.0, 30.0),         # amplitude LFO rate
        ("depth", 0.2, 0.0, 1.0),       # 0 untouched .. 1 fully gated
    ),
    "wobble": (
        ("hz", 0.7, 0.0, 8.0),          # rotation LFO rate
        ("depth", 0.35, 0.0, 1.5),      # peak deflection (radians)
    ),
    "matrix": (
        ("a", 1.0, -2.0, 2.0),          # L' = a*L + b*R
        ("b", 0.0, -2.0, 2.0),
        ("c", 0.0, -2.0, 2.0),          # R' = c*L + d*R
        ("d", 1.0, -2.0, 2.0),
    ),
    "chandelay": (
        ("ms", 5.0, 0.0, 50.0),         # delay one channel: mono grows lissajous
        ("channel", 1.0, 0.0, 1.0),     # 0 = left, 1 = right
    ),
}
PARAMETERS_PER_STAGE = 4


def clamp(value, low, high):
    return max(low, min(high, float(value)))


def canonical_stages(raw_stages):
    """Validated [(op_name, [p0..p3])] from JSON stage dicts. Unknown ops
    and malformed values raise ValueError with a pointed message."""
    if not isinstance(raw_stages, list) or not raw_stages:
        raise ValueError("stages: expected a non-empty list")
    if len(raw_stages) > MAX_STAGES:
        raise ValueError(f"stages: at most {MAX_STAGES} allowed")
    stages = []
    for index, stage in enumerate(raw_stages):
        if not isinstance(stage, dict) or "op" not in stage:
            raise ValueError(f"stages[{index}]: expected {{'op': …}}")
        op = stage["op"]
        if op not in OPERATIONS:
            raise ValueError(
                f"stages[{index}]: unknown op '{op}' "
                f"(known: {', '.join(sorted(OPERATIONS))})")
        parameters = []
        for key, default, low, high in OPERATIONS[op]:
            value = stage.get(key, default)
            try:
                parameters.append(clamp(value, low, high))
            except (TypeError, ValueError):
                raise ValueError(
                    f"stages[{index}].{key}: expected a number") from None
        parameters += [0.0] * (PARAMETERS_PER_STAGE - len(parameters))
        stages.append((op, parameters))
    return stages


def stages_to_json(stages):
    """The JSON stage dicts for canonical [(op, params)] pairs."""
    raw = []
    for op, parameters in stages:
        stage = {"op": op}
        for (key, _default, _low, _high), value in zip(OPERATIONS[op],
                                                       parameters):
            stage[key] = round(float(value), 6)
        raw.append(stage)
    return raw


def load(path):
    """(name, author, canonical stages) from a .phoskit file."""
    with open(path) as kit_file:
        document = json.load(kit_file)
    if not isinstance(document, dict) or "stages" not in document:
        raise ValueError("not a phoskit document")
    version = document.get("phoskit", 1)
    if int(version) > FORMAT_VERSION:
        raise ValueError(f"phoskit version {version} is newer than this "
                         "Phosphor understands")
    name = str(document.get("name")
               or os.path.splitext(os.path.basename(path))[0])
    author = str(document.get("author") or "")
    return name, author, canonical_stages(document["stages"])


def save(path, name, author, stages):
    """Write a .phoskit; returns the path."""
    directory = os.path.dirname(path)
    if directory:
        os.makedirs(directory, exist_ok=True)
    document = {"phoskit": FORMAT_VERSION, "name": name}
    if author:
        document["author"] = author
    document["stages"] = stages_to_json(stages)
    with open(path, "w") as kit_file:
        json.dump(document, kit_file, indent=2)
        kit_file.write("\n")
    return path


def install(source_path):
    """Copy a kit into the user kit directory (drag-drop import); returns
    the installed path. Validates before copying — a broken kit never lands."""
    load(source_path)
    os.makedirs(KIT_DIRECTORY, exist_ok=True)
    destination = os.path.join(KIT_DIRECTORY, os.path.basename(source_path))
    if os.path.abspath(source_path) != os.path.abspath(destination):
        with open(source_path, "rb") as source, \
                open(destination, "wb") as sink:
            sink.write(source.read())
    return destination


def available_kits():
    """[(display_name, path)] across the user and system kit directories,
    user kits shadowing same-named system ones."""
    found = {}
    for directory in (BUNDLED_KIT_DIRECTORY, SYSTEM_KIT_DIRECTORY,
                      KIT_DIRECTORY):
        try:
            names = sorted(os.listdir(directory), key=str.casefold)
        except OSError:
            continue
        for file_name in names:
            if file_name.endswith(".phoskit"):
                found[file_name] = os.path.join(directory, file_name)
    kits = []
    for file_name, path in sorted(found.items(), key=lambda kv: kv[0].casefold()):
        try:
            name, author, _stages = load(path)
        except (OSError, ValueError):
            continue
        label = f"{name} — {author}" if author else name
        kits.append((label, path))
    return kits
