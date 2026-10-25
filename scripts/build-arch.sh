#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build the Arch package of HEAD in an Arch container (used by the release workflow).
# Usage: build-arch.sh IMAGE X.Y.Z OUTDIR
#   IMAGE  archlinux / Arch Linux ARM image (its platform decides the package arch)
#   OUTDIR receives obsbot4linux-X.Y.Z-1-<arch>.pkg.tar.zst
set -eu
image=${1:?usage: build-arch.sh IMAGE X.Y.Z OUTDIR}
ver=${2:?}
out=$(mkdir -p "${3:?}" && cd "$3" && pwd)
root=$(cd "$(dirname "$0")/.." && pwd)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git -C "$root" archive --prefix="obsbot4linux-$ver/" -o "$work/obsbot4linux-$ver.tar.gz" HEAD
cp "$root/packaging/arch/PKGBUILD" "$work/PKGBUILD"
sed -i "s/^pkgver=.*/pkgver=$ver/" "$work/PKGBUILD"
chmod -R a+rwX "$work"

docker run --rm -v "$work:/work" -e VER="$ver" "$image" sh -ec '
  pacman -Syu --noconfirm --needed cargo
  useradd -m builder
  chown -R builder /work
  su builder -c "cd /work && OBSBOT4LINUX_TARBALL=/work/obsbot4linux-$VER.tar.gz PKGDEST=/work/out makepkg --noconfirm"
  chmod -R a+rwX /work
'
cp "$work"/out/obsbot4linux-"$ver"-1-*.pkg.tar.zst "$out"
