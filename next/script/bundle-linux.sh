#!/usr/bin/env bash
# Package only the independent Next Alpha identity.
set -euo pipefail
: "${VERSION:?VERSION must be the allocated Next Alpha version}"
[[ "$VERSION" =~ ^0\.2\.0-alpha\.[1-9][0-9]*$ ]]
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
binary=next/target/release/drifting
test -f "$binary"
for face in DriftingSans-Regular.ttf DriftingSans-Medium.ttf DriftingSans-SemiBold.ttf DriftingSerif-Regular.ttf DriftingSerif-Bold.ttf SarasaMonoSC-Regular.ttf OFL-1.1.txt; do
  test -f "next/.cache/fonts/$face"
done
mkdir -p out/release out/deb/DEBIAN out/deb/opt/drifting-next-alpha/fonts out/deb/usr/bin out/deb/usr/share/applications out/deb/usr/share/icons/hicolor/512x512/apps out/bin
install -m 0755 "$binary" out/deb/opt/drifting-next-alpha/drifting-next-alpha
cp next/.cache/fonts/* out/deb/opt/drifting-next-alpha/fonts/
cat >out/deb/usr/bin/drifting-next-alpha <<'WRAPPER'
#!/bin/sh
exec /opt/drifting-next-alpha/drifting-next-alpha "$@"
WRAPPER
chmod +x out/deb/usr/bin/drifting-next-alpha
cat >out/deb/DEBIAN/control <<CONTROL
Package: drifting-next-alpha
Version: $VERSION
Section: editors
Priority: optional
Architecture: amd64
Maintainer: Drifting contributors
Depends: libc6 (>= 2.39), libasound2t64, libdbus-1-3, libegl1, libfontconfig1, libvulkan1, libwayland-client0, libx11-xcb1, libxkbcommon0, libxkbcommon-x11-0
Recommends: mesa-vulkan-drivers
Description: Drifting Next Alpha writing client
 Independent Next Alpha installation; does not replace Drifting 0.1.
CONTROL
cat >out/deb/usr/share/applications/cc.drifting.next.alpha.desktop <<'DESKTOP'
[Desktop Entry]
Name=Drifting Next Alpha
Exec=drifting-next-alpha
Icon=cc.drifting.next.alpha
Type=Application
Categories=Office;WordProcessor;
Terminal=false
DESKTOP
cp src/assets/icon.png out/deb/usr/share/icons/hicolor/512x512/apps/cc.drifting.next.alpha.png
printf -v deb 'Drifting_Next_%s_linux_x86_64.deb' "$VERSION"
printf -v appimage 'Drifting_Next_%s_linux_x86_64.AppImage' "$VERSION"
dpkg-deb --root-owner-group --build out/deb "out/release/$deb"

# The downloaded packaging tool is verified before execution. A changed
# upstream continuous artifact fails closed until its pin is reviewed.
curl --fail --location --retry 3 --output out/linuxdeploy https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
echo 'df908b19f1ed1cdafa79508d415aa12d6c56ce4d56d4149aa8c09286718706e5  out/linuxdeploy' | sha256sum --check -
chmod +x out/linuxdeploy
mkdir -p out/appimage/usr/bin/fonts
cp next/.cache/fonts/* out/appimage/usr/bin/fonts/
install -m 0755 "$binary" out/bin/drifting-next-alpha
APPIMAGE_EXTRACT_AND_RUN=1 LDAI_NO_APPSTREAM=1 LDAI_OUTPUT="$PWD/out/release/$appimage" LINUXDEPLOY_OUTPUT_VERSION="$VERSION" \
  out/linuxdeploy --appdir out/appimage \
    --executable out/bin/drifting-next-alpha \
    --desktop-file out/deb/usr/share/applications/cc.drifting.next.alpha.desktop \
    --icon-file out/deb/usr/share/icons/hicolor/512x512/apps/cc.drifting.next.alpha.png \
    --output appimage

sudo dpkg -i "out/release/$deb"
trap 'sudo dpkg -r drifting-next-alpha >/dev/null 2>&1 || true' EXIT
bash next/script/check-next-linux.sh /opt/drifting-next-alpha/drifting-next-alpha
chmod +x "out/release/$appimage"
APPIMAGE_EXTRACT_AND_RUN=1 bash next/script/check-next-linux.sh "$PWD/out/release/$appimage"
