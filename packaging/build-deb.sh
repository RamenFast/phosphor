#!/usr/bin/env bash
# Build a Debian package for Phosphor straight from the working tree.
#
#   packaging/build-deb.sh        -> packaging/dist/phosphor_<version>_all.deb
#
# The version comes from APPLICATION_VERSION in phosphor.py, so bumping it
# there is the whole release dance.
set -euo pipefail

project_directory="$(cd "$(dirname "$0")/.." && pwd)"
packaging_directory="$project_directory/packaging"
package_name="phosphor"
version="$(grep -oP 'APPLICATION_VERSION = "\K[^"]+' "$project_directory/phosphor.py")"

staging_directory="$(mktemp -d)"
trap 'rm -rf "$staging_directory"' EXIT

install -d \
    "$staging_directory/DEBIAN" \
    "$staging_directory/usr/lib/phosphor" \
    "$staging_directory/usr/bin" \
    "$staging_directory/usr/share/applications" \
    "$staging_directory/usr/share/icons/hicolor/scalable/apps"

install -m 644 "$project_directory"/phosphor*.py "$staging_directory/usr/lib/phosphor/"

# Native signal core: build it if cargo is around (or reuse a previous
# build). The package stays installable without it — Python falls back.
architecture="all"
native_library="$project_directory/core/target/release/libphosphor_core.so"
if command -v cargo >/dev/null 2>&1; then
    (cd "$project_directory/core" && cargo build --release --quiet)
fi
if [ -f "$native_library" ]; then
    install -m 644 "$native_library" "$staging_directory/usr/lib/phosphor/"
    architecture="$(dpkg --print-architecture)"
fi
# Starter signal kits (.phoskit postcards) — user kits shadow these
if [ -d "$project_directory/kits" ]; then
    install -d "$staging_directory/usr/share/phosphor/kits"
    install -m 644 "$project_directory"/kits/*.phoskit \
        "$staging_directory/usr/share/phosphor/kits/"
fi

# Studio starter scenes + the phosphor-studio manpage (when scdoc is here)
if [ -d "$project_directory/scenes" ]; then
    install -d "$staging_directory/usr/share/phosphor/scenes"
    install -m 644 "$project_directory"/scenes/*.scene.json \
        "$staging_directory/usr/share/phosphor/scenes/"
fi
if command -v scdoc >/dev/null 2>&1; then
    install -d "$staging_directory/usr/share/man/man1"
    scdoc < "$project_directory/docs/phosphor-studio.1.scd" \
        | gzip -9 > "$staging_directory/usr/share/man/man1/phosphor-studio.1.gz"
fi

install -m 644 "$project_directory/phosphor-scope.svg" "$staging_directory/usr/lib/phosphor/"
install -m 644 "$project_directory/phosphor-scope.svg" \
    "$staging_directory/usr/share/icons/hicolor/scalable/apps/phosphor-scope.svg"
install -m 644 "$packaging_directory/phosphor.desktop" \
    "$staging_directory/usr/share/applications/"
install -d "$staging_directory/usr/share/doc/phosphor"
install -m 644 "$project_directory/LICENSE" \
    "$staging_directory/usr/share/doc/phosphor/copyright"

cat > "$staging_directory/usr/bin/phosphor" <<'LAUNCHER'
#!/bin/sh
exec python3 /usr/lib/phosphor/phosphor.py "$@"
LAUNCHER
chmod 755 "$staging_directory/usr/bin/phosphor"

cat > "$staging_directory/usr/bin/phosphor-studio" <<'LAUNCHER'
#!/bin/sh
exec python3 /usr/lib/phosphor/phosphor_studio.py "$@"
LAUNCHER
chmod 755 "$staging_directory/usr/bin/phosphor-studio"

installed_size_kb="$(du -sk "$staging_directory/usr" | cut -f1)"
cat > "$staging_directory/DEBIAN/control" <<CONTROL
Package: $package_name
Version: $version
Section: sound
Priority: optional
Architecture: $architecture
Installed-Size: $installed_size_kb
Depends: python3, python3-gi, python3-gi-cairo, gir1.2-gtk-3.0, pulseaudio-utils
Recommends: ffmpeg, python3-numpy
Maintainer: Ben <2bmillerb@gmail.com>
Description: CRT-style XY oscilloscope for desktop audio
 Phosphor draws what your PC plays the way an analog scope would:
 XY mode for oscilloscope music, a goniometer for stereo width,
 plus waveform and spectrum views. It simulates P7 phosphor decay
 on the GPU (OpenGL) or CPU (cairo), captures whole outputs, single
 applications, microphones, or audio files, and exports snapshots
 and mp4 clips with sound.
CONTROL

output_directory="$packaging_directory/dist"
mkdir -p "$output_directory"
output_file="$output_directory/${package_name}_${version}_${architecture}.deb"
dpkg-deb --build --root-owner-group "$staging_directory" "$output_file" > /dev/null
echo "built: $output_file"
dpkg-deb --info "$output_file" | sed -n '1,12p'
