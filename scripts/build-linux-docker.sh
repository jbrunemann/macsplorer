#!/bin/sh
# Builds the Linux tarball in an Ubuntu 22.04 container, for building from a
# Mac. Output: target/linux-docker/macsplorer-linux-<arch>.tar.gz. Needs a
# running Docker. The container is native to this machine (aarch64 on Apple
# Silicon); pass a platform for the other one, which runs emulated and slow.
# Ubuntu 22.04 keeps the glibc requirement low (2.35).
# Usage: scripts/build-linux-docker.sh [linux/amd64|linux/arm64]
set -eu
cd "$(dirname "$0")/.."

mkdir -p target/linux-docker
docker run --rm ${1:+--platform "$1"} \
    -v "$PWD":/src:ro \
    -v macsplorer-linux-target:/target \
    -v macsplorer-linux-cargo:/root/.cargo \
    -v macsplorer-linux-rustup:/root/.rustup \
    -v "$PWD/target/linux-docker":/out \
    -e CARGO_TARGET_DIR=/target \
    -e DEBIAN_FRONTEND=noninteractive \
    ubuntu:22.04 sh -eu -c '
        apt-get update -qq
        apt-get install -y -qq curl build-essential pkg-config ca-certificates >/dev/null
        [ -x /root/.cargo/bin/rustup ] || curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
        . /root/.cargo/env
        rustup set profile minimal
        rustup show active-toolchain >/dev/null 2>&1 || rustup default stable
        cd /src
        scripts/bundle-linux.sh
        cp /target/release/macsplorer-linux-*.tar.gz /out/
    '
ls target/linux-docker/*.tar.gz
