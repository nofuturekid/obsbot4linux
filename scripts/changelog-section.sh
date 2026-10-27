#!/bin/sh
# Print the CHANGELOG.md section for a release tag (vX.Y.Z or X.Y.Z).
# Fails unless Cargo.toml and packaging/arch/PKGBUILD have that version and CHANGELOG.md has
# a "## X.Y.Z" (or "## [X.Y.Z]") heading with content below it. Used by the release workflow.
set -eu
cd "$(dirname "$0")/.."

ver=${1:?usage: changelog-section.sh vX.Y.Z}
ver=${ver#v}
case $ver in
*[!0-9.]* | *..* | .* | *. | *.*.*.*) echo "not a X.Y.Z version: $ver" >&2; exit 1 ;;
*.*.*) ;;
*) echo "not a X.Y.Z version: $ver" >&2; exit 1 ;;
esac

crate=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
if [ "$crate" != "$ver" ]; then
  echo "Cargo.toml has version $crate, tag is $ver" >&2
  exit 1
fi

pkgver=$(sed -n 's/^pkgver=//p' packaging/arch/PKGBUILD)
if [ "$pkgver" != "$ver" ]; then
  echo "packaging/arch/PKGBUILD has pkgver $pkgver, tag is $ver" >&2
  exit 1
fi

notes=$(awk -v v="$ver" '
  /^## / { on = ($2 == v || $2 == "[" v "]"); next }
  on
' CHANGELOG.md)
if [ -z "$(printf %s "$notes" | tr -d ' \n')" ]; then
  echo "CHANGELOG.md has no non-empty \"## $ver\" section" >&2
  exit 1
fi
printf '%s\n' "$notes"
