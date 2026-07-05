#!/usr/bin/env bash
# Build the RPM (Fedora/openSUSE) from the same tree as the .deb.
#
#   packaging/build-rpm.sh   -> packaging/dist/phosphor-<v>-1.<arch>.rpm
#
# Uses cargo-generate-rpm (asset table in crates/phosphor-app/
# Cargo.toml). Shared-library Requires are auto-detected from the
# ELF; ffmpeg is declared explicitly (clips/render mux). Built and
# `rpm --test`-verified on a Debian-family host — reports from real
# Fedora installs welcome.
set -euo pipefail

project_directory="$(cd "$(dirname "$0")/.." && pwd)"
cd "$project_directory"

cargo build --release --quiet -p phosphor-app

mkdir -p target/rpm-stage
scdoc < docs/phosphor.1.scd | gzip -9n > target/rpm-stage/phosphor.1.gz

PATH="$HOME/.cargo/bin:$PATH" cargo generate-rpm \
    -p crates/phosphor-app \
    --output packaging/dist/

built="$(ls -t packaging/dist/phosphor-*.rpm | head -1)"
echo "built: $built"
rpm -qpi "$built" | sed -n '1,10p'
echo "--- payload:"
rpm -qpl "$built"
