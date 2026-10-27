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

# The container runs as root, builds as "builder" and finally hands /work back to the
# calling user (HOST_UID/HOST_GID): their uids differ on CI runners, and without the
# chown the cleanup trap above cannot delete the files (and /tmp is sticky).
# PKGEXT is set because Arch Linux ARM's makepkg.conf still defaults to .pkg.tar.xz.
# Arch does not support partial upgrades, so installing cargo needs -Syu. That pulls the
# current packages from the mirrors: the image digest pins only the starting point, the
# build is not reproducible.
docker run --rm -v "$work:/work" -e VER="$ver" -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  "$image" sh -ec '
  trap "chown -R \$HOST_UID:\$HOST_GID /work" EXIT
  pacman -Syu --noconfirm --needed --disable-sandbox cargo
  useradd -m builder
  chown -R builder /work
  su builder -c "cd /work && OBSBOT4LINUX_TARBALL=/work/obsbot4linux-$VER.tar.gz PKGDEST=/work/out PKGEXT=.pkg.tar.zst makepkg --noconfirm"
'

pkg=$(ls "$work"/out/obsbot4linux-"$ver"-1-*.pkg.tar.zst 2>/dev/null || true)
if [ -z "$pkg" ]; then
  echo "build-arch.sh: no obsbot4linux-$ver-1-*.pkg.tar.zst in /work/out; it contains:" >&2
  ls -la "$work/out" >&2 || true
  exit 1
fi
cp $pkg "$out"
