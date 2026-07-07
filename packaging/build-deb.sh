#!/usr/bin/env bash
# Build the Debian package for Phosphor (the compiled Rust binary)
# straight from the working tree.
#
#   packaging/build-deb.sh          -> packaging/dist/phosphor_<version>_<arch>.deb
#
# Pre-release builds append a ~ suffix (sorts BEFORE the release, so
# wave debs upgraded cleanly into 4.0.0). The v3 python-tree packaging
# lives in git history — last shipped as phosphor_3.5.0_amd64.deb.
set -euo pipefail

project_directory="$(cd "$(dirname "$0")/.." && pwd)"
packaging_directory="$project_directory/packaging"
package_name="phosphor"
VERSION="4.1.0"

# The binary: build fresh, package a stripped copy.
(cd "$project_directory" && cargo build --release --quiet -p phosphor-app)
binary="$project_directory/target/release/phosphor"
[ -f "$binary" ] || { echo "no release binary at $binary" >&2; exit 1; }
architecture="$(dpkg --print-architecture)"

staging_directory="$(mktemp -d)"
trap 'rm -rf "$staging_directory"' EXIT

install -d \
    "$staging_directory/DEBIAN" \
    "$staging_directory/usr/bin" \
    "$staging_directory/usr/share/applications" \
    "$staging_directory/usr/share/icons/hicolor/scalable/apps" \
    "$staging_directory/usr/share/icons/hicolor/256x256/apps" \
    "$staging_directory/usr/share/phosphor/kits" \
    "$staging_directory/usr/share/doc/phosphor"

install -m 755 "$binary" "$staging_directory/usr/bin/phosphor"
strip --strip-unneeded "$staging_directory/usr/bin/phosphor"

# Starter signal kits (.phoskit postcards) — user kits shadow these
install -m 644 "$project_directory"/kits/*.phoskit \
    "$staging_directory/usr/share/phosphor/kits/"

# The 4-panel icon (one scope shape per panel), name kept from v3 so
# existing launchers/docks keep their icon
install -m 644 "$packaging_directory/phosphor4-scope.svg" \
    "$staging_directory/usr/share/icons/hicolor/scalable/apps/phosphor-scope.svg"
install -m 644 "$packaging_directory/phosphor4-scope-256.png" \
    "$staging_directory/usr/share/icons/hicolor/256x256/apps/phosphor-scope.png"
install -m 644 "$packaging_directory/phosphor.desktop" \
    "$staging_directory/usr/share/applications/"
install -m 644 "$project_directory/LICENSE" \
    "$staging_directory/usr/share/doc/phosphor/copyright"

# Manpage (scdoc is a build-time dependency, like the v3 packaging)
install -d "$staging_directory/usr/share/man/man1"
scdoc < "$project_directory/docs/phosphor.1.scd" \
    | gzip -9n > "$staging_directory/usr/share/man/man1/phosphor.1.gz"

installed_size_kb="$(du -sk "$staging_directory/usr" | cut -f1)"
cat > "$staging_directory/DEBIAN/control" <<CONTROL
Package: $package_name
Version: $VERSION
Section: sound
Priority: optional
Architecture: $architecture
Installed-Size: $installed_size_kb
Depends: libpipewire-0.3-0 (>= 0.3.65), libc6, libgcc-s1
Recommends: ffmpeg, libvulkan1, pulseaudio-utils
Maintainer: Ben <2bmillerb@gmail.com>
Description: CRT-style XY oscilloscope for desktop audio
 Phosphor draws what your PC plays the way an analog scope would:
 XY mode for oscilloscope music, a goniometer for stereo width, 3D
 delay-embedding views, waveform and spectrum. One Rust engine
 simulates P7 phosphor decay on the GPU (wgpu) or CPU (SIMD),
 captures whole outputs, single applications, microphones, or audio
 files natively over PipeWire, plays signal postcards (.phos), wears
 transform kits (.phoskit), lets you draw shapes the scope plays
 back, and exports snapshots and mp4 clips with sound.
CONTROL

output_directory="$packaging_directory/dist"
mkdir -p "$output_directory"
output_file="$output_directory/${package_name}_${VERSION}_${architecture}.deb"
dpkg-deb --build --root-owner-group "$staging_directory" "$output_file" > /dev/null
echo "built: $output_file"
dpkg-deb --info "$output_file" | sed -n '1,14p'
