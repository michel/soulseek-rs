#!/bin/sh
# Installs the latest soulseek-rs release.
#
#   curl -fsSL https://re-invention.nl/soulseek-rs/install.sh | sh
#
# Uses Homebrew when it is on the PATH (macOS and Linux), otherwise downloads
# the prebuilt binary for this platform from GitHub releases, verifies its
# sha256, and puts it in /usr/local/bin or ~/.local/bin.
#
# SOULSEEK_RS_INSTALL_DIR overrides where the binary lands.
set -eu

REPO="michel/soulseek-rs"

say() { printf '%s\n' "$*"; }
die() {
  printf 'install.sh: %s\n' "$*" >&2
  exit 1
}

case "$(uname -s)" in
  Darwin) os="apple-darwin" ;;
  Linux) os="unknown-linux-musl" ;;
  MINGW* | MSYS* | CYGWIN* | Windows_NT)
    die "no install script for Windows: take the pc-windows-msvc zip from https://github.com/$REPO/releases/latest, or run 'cargo install soulseek-rs'" ;;
  *) die "unsupported OS '$(uname -s)': see https://github.com/$REPO/releases/latest" ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) arch="x86_64" ;;
  arm64 | aarch64) arch="aarch64" ;;
  *) die "unsupported architecture '$(uname -m)': releases cover x86_64 and aarch64" ;;
esac
target="$arch-$os"

if command -v brew >/dev/null 2>&1; then
  say "Homebrew found: installing michel/tap/soulseek-rs (brew also handles upgrades)"
  exec brew install michel/tap/soulseek-rs
fi

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -qO- "$1"; }
else
  die "need curl or wget"
fi

tag=$(fetch "https://api.github.com/repos/$REPO/releases/latest" |
  sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
[ -n "$tag" ] || die "could not resolve the latest release; downloads are at https://github.com/$REPO/releases/latest"

archive="soulseek-rs-$tag-$target.tar.gz"
base_url="https://github.com/$REPO/releases/download/$tag"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

say "Downloading $archive"
fetch "$base_url/$archive" >"$tmp/$archive"
fetch "$base_url/soulseek-rs-$tag-$target.sha256" >"$tmp/expected.sha256"

if command -v sha256sum >/dev/null 2>&1; then
  got=$(sha256sum "$tmp/$archive")
else
  got=$(shasum -a 256 "$tmp/$archive")
fi
got=${got%% *}
want=$(cut -d' ' -f1 <"$tmp/expected.sha256")
[ "$got" = "$want" ] || die "sha256 mismatch for $archive: expected $want, got $got"

tar -xzf "$tmp/$archive" -C "$tmp"
[ -f "$tmp/soulseek-rs" ] || die "archive $archive did not contain a soulseek-rs binary"

dir="${SOULSEEK_RS_INSTALL_DIR:-}"
if [ -z "$dir" ]; then
  dir="/usr/local/bin"
  { [ -d "$dir" ] && [ -w "$dir" ]; } || dir="$HOME/.local/bin"
fi
mkdir -p "$dir"

# mv over a copy, not cp over the target: replacing a running binary in place
# fails with "text file busy" on Linux.
cp "$tmp/soulseek-rs" "$dir/.soulseek-rs.new"
chmod 755 "$dir/.soulseek-rs.new"
mv -f "$dir/.soulseek-rs.new" "$dir/soulseek-rs"

say "Installed $("$dir/soulseek-rs" --version) to $dir/soulseek-rs"
case ":$PATH:" in
  *:"$dir":*) ;;
  *) say "note: $dir is not on your PATH; add it with: export PATH=\"$dir:\$PATH\"" ;;
esac
