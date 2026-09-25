#!/usr/bin/env bash
# Builds a Fedora RPM inside a Fedora container: target/rpm/pigeon-<version>-1.fc<N>.x86_64.rpm
# Usage: packaging/build-rpm.sh [fedora-release]   (default: 43)
set -euo pipefail
cd "$(dirname "$0")/.."

FEDORA=${1:-43}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

# crates are cached in a named volume so rebuilds don't download everything again
docker run --rm \
    -v "$PWD":/src:Z -w /src \
    -v pigeon-cargo-cache:/root/.cargo/registry \
    -e VERSION="$VERSION" -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
    "fedora:$FEDORA" bash -euo pipefail -c '
        dnf -y -q install cargo rust gtk4-devel libadwaita-devel rpm-build >/dev/null
        export CARGO_TARGET_DIR=/src/target/fedora
        cargo build --release --locked

        top=/src/target/fedora/rpmbuild
        rm -rf "$top" && mkdir -p "$top/SOURCES"
        cp "$CARGO_TARGET_DIR/release/pigeon" data/dev.pigeon.Pigeon.desktop LICENSE README.md "$top/SOURCES/"
        "$CARGO_TARGET_DIR/release/pigeon" --export-icon "$top/SOURCES/dev.pigeon.Pigeon.svg"
        rpmbuild -bb --quiet --define "_topdir $top" --define "pigeon_version $VERSION" packaging/pigeon.spec

        mkdir -p /src/target/rpm
        cp "$top"/RPMS/*/*.rpm /src/target/rpm/
        chown -R "$HOST_UID:$HOST_GID" /src/target/fedora /src/target/rpm
    '
ls target/rpm/*.rpm
