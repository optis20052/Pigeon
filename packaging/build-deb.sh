#!/usr/bin/env bash
# Builds a Debian package: target/deb/pigeon_<version>_<arch>.deb
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(dpkg --print-architecture)
OUT=target/deb
STAGE="$OUT/pigeon_${VERSION}_${ARCH}"
rm -rf "$STAGE"

install -Dm755 target/release/pigeon "$STAGE/usr/bin/pigeon"
install -Dm644 data/dev.pigeon.Pigeon.desktop "$STAGE/usr/share/applications/dev.pigeon.Pigeon.desktop"
# the icon is generated from code (src/ui/app_icon.rs); this writes the default variant
install -d -m755 "$STAGE/usr/share/icons" "$STAGE/usr/share/icons/hicolor" "$STAGE/usr/share/icons/hicolor/scalable" "$STAGE/usr/share/icons/hicolor/scalable/apps"
target/release/pigeon --export-icon "$STAGE/usr/share/icons/hicolor/scalable/apps/dev.pigeon.Pigeon.svg"
chmod 644 "$STAGE/usr/share/icons/hicolor/scalable/apps/dev.pigeon.Pigeon.svg"
install -Dm644 README.md "$STAGE/usr/share/doc/pigeon/README.md"
{
    printf 'Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\n'
    printf 'Upstream-Name: Pigeon\n\n'
    printf 'Files: *\n'
    printf 'Copyright: %s\n' "$(sed -n 's/^Copyright (c) //p' LICENSE)"
    printf 'License: MIT\n'
    # license text, indented as the copyright format requires (blank lines become " .")
    sed -n '/^Permission is hereby granted/,$p' LICENSE | sed 's/^$/./; s/^/ /'
} > "$STAGE/usr/share/doc/pigeon/copyright"
chmod 644 "$STAGE/usr/share/doc/pigeon/copyright"

# shared-library dependencies of the binary (dpkg-shlibdeps needs a debian/control to run)
SHLIBS=$(mktemp -d)
mkdir -p "$SHLIBS/debian"
printf 'Source: pigeon\n\nPackage: pigeon\nArchitecture: any\n' > "$SHLIBS/debian/control"
DEPENDS=$(cd "$SHLIBS" && dpkg-shlibdeps -O -e "$OLDPWD/$STAGE/usr/bin/pigeon" 2>/dev/null | sed -n 's/^shlibs:Depends=//p')
rm -rf "$SHLIBS"

mkdir -p "$STAGE/DEBIAN"
cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: pigeon
Version: $VERSION
Architecture: $ARCH
Maintainer: Ali <ali@spentys.com>
Installed-Size: $(du -sk "$STAGE/usr" | cut -f1)
Depends: $DEPENDS
Section: devel
Priority: optional
Description: Fast, native API client for GNOME
 Pigeon builds, sends and tests HTTP requests. It organizes requests in
 projects and collections, supports environments and variables, imports
 OpenAPI / Swagger specs and cURL commands, and generates code snippets.
 Built with Rust, GTK4 and libadwaita.
CONTROL

dpkg-deb --build --root-owner-group "$STAGE" "$OUT" >/dev/null
echo "$OUT/pigeon_${VERSION}_${ARCH}.deb"
