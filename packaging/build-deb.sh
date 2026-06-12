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
install -m 644 "$project_directory/phosphor-scope.svg" "$staging_directory/usr/lib/phosphor/"
install -m 644 "$project_directory/phosphor-scope.svg" \
    "$staging_directory/usr/share/icons/hicolor/scalable/apps/phosphor-scope.svg"
install -m 644 "$packaging_directory/phosphor.desktop" \
    "$staging_directory/usr/share/applications/"

cat > "$staging_directory/usr/bin/phosphor" <<'LAUNCHER'
#!/bin/sh
exec python3 /usr/lib/phosphor/phosphor.py "$@"
LAUNCHER
chmod 755 "$staging_directory/usr/bin/phosphor"

installed_size_kb="$(du -sk "$staging_directory/usr" | cut -f1)"
cat > "$staging_directory/DEBIAN/control" <<CONTROL
Package: $package_name
Version: $version
Section: sound
Priority: optional
Architecture: all
Installed-Size: $installed_size_kb
Depends: python3, python3-gi, python3-gi-cairo, gir1.2-gtk-3.0, pulseaudio-utils
Recommends: ffmpeg
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
output_file="$output_directory/${package_name}_${version}_all.deb"
dpkg-deb --build --root-owner-group "$staging_directory" "$output_file" > /dev/null
echo "built: $output_file"
dpkg-deb --info "$output_file" | sed -n '1,12p'
