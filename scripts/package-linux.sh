#!/bin/sh
set -eu
umask 022

[ $# -ge 4 ] || {
  echo "usage: $0 <binary> <version> <target-triple> <out-dir> [<package-basename>]" >&2
  exit 2
}

NFPM_VERSION=2.47.0

binary=$1
PKG_VERSION=$2
target=$3
out=$4
package=${5:-soulseek-rs}

case "$target" in
  x86_64-*) PKG_ARCH=amd64 rpm_arch=x86_64 ;;
  aarch64-*) PKG_ARCH=arm64 rpm_arch=aarch64 ;;
  *) echo "no package architecture for $target" >&2; exit 1 ;;
esac

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) nfpm_asset=Linux_x86_64 nfpm_sha256=0660ca602b2d2d2ae4781a06c692b3eeb9d437ffea05b831d76e41f4a3188783 ;;
  Linux-aarch64) nfpm_asset=Linux_arm64 nfpm_sha256=1c0f5f2999b9a974bfb04fdb0cc3306096de530ac5dbb25d739cc5f5219c919c ;;
  *) echo "no pinned nfpm build for this machine" >&2; exit 1 ;;
esac

root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
stage=$root/target/nfpm
rm -rf "$stage"
mkdir -p "$stage" "$out"
out=$(cd "$out" && pwd)

cp "$binary" "$stage/soulseek-rs"
"$binary" completions print bash > "$stage/soulseek-rs.bash"
"$binary" completions print zsh > "$stage/_soulseek-rs"
"$binary" completions print fish > "$stage/soulseek-rs.fish"
"$binary" man > "$stage/soulseek-rs.1"
gzip -9n "$stage/soulseek-rs.1"

archive=nfpm_${NFPM_VERSION}_$nfpm_asset.tar.gz
curl -fsSL -o "$work/$archive" \
  "https://github.com/goreleaser/nfpm/releases/download/v$NFPM_VERSION/$archive"
echo "$nfpm_sha256  $work/$archive" | sha256sum -c - >/dev/null
tar -xzf "$work/$archive" -C "$work" nfpm

export PKG_ARCH PKG_VERSION
cd "$root"
"$work/nfpm" package --config packaging/nfpm.yaml --packager deb --target "$out/$package-$PKG_ARCH.deb"
"$work/nfpm" package --config packaging/nfpm.yaml --packager rpm --target "$out/$package-$rpm_arch.rpm"
