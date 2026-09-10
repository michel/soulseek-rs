#!/bin/sh
# Installs soulseek-rs from GitHub releases.
#
# Latest stable release:
#   curl -fsSL https://re-invention.nl/soulseek-rs/install.sh | sh
#
# Latest successful build from develop:
#   curl -fsSL https://raw.githubusercontent.com/michel/soulseek-rs/develop/website/public/install.sh | sh -s -- --nightly
#
# Stable installs use Homebrew when it is on PATH (macOS and Linux). Nightly
# installs always download the prebuilt binary for this platform. Direct
# downloads are sha256-verified and replace an existing binary atomically.
#
# SOULSEEK_RS_INSTALL_DIR overrides where the binary lands.
set -eu

REPO="michel/soulseek-rs"

say() { printf '%s\n' "$*"; }
die() {
  printf 'install.sh: %s\n' "$*" >&2
  exit 1
}
usage() {
  cat <<'EOF'
Usage: install.sh [--nightly]

Install the latest stable soulseek-rs release. Options:
  --nightly  Install the latest successful build from the develop branch.
  -h, --help Show this help.

When piping the script, pass the option after `sh -s --`:
  curl -fsSL https://raw.githubusercontent.com/michel/soulseek-rs/develop/website/public/install.sh | sh -s -- --nightly

SOULSEEK_RS_INSTALL_DIR overrides the destination directory.
EOF
}

channel="stable"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --nightly) channel="nightly" ;;
    -h | --help)
      usage
      exit 0
      ;;
    --)
      shift
      [ "$#" -eq 0 ] || die "unexpected argument '$1' (try --help)"
      break
      ;;
    *) die "unknown option '$1' (try --help)" ;;
  esac
  shift
done

if [ "$channel" = "nightly" ]; then
  tag="nightly"
  release_page="https://github.com/$REPO/releases/tag/nightly"
else
  tag=""
  release_page="https://github.com/$REPO/releases/latest"
fi

system=$(uname -s)
case "$system" in
  Darwin) os="apple-darwin" ;;
  Linux) os="unknown-linux-musl" ;;
  MINGW* | MSYS* | CYGWIN* | Windows_NT)
    die "no install script for Windows: take the pc-windows-msvc zip from $release_page, or run 'cargo install soulseek-rs'" ;;
  *) die "unsupported OS '$system': see $release_page" ;;
esac

machine=$(uname -m)
case "$machine" in
  x86_64 | amd64) arch="x86_64" ;;
  arm64 | aarch64) arch="aarch64" ;;
  *) die "unsupported architecture '$machine': builds cover x86_64 and aarch64" ;;
esac
target="$arch-$os"

if [ "$channel" = "stable" ] && [ -z "${SOULSEEK_RS_INSTALL_DIR:-}" ] &&
  command -v brew >/dev/null 2>&1; then
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

command -v tar >/dev/null 2>&1 || die "need tar to unpack the download"
if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1"; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1"; }
else
  die "need sha256sum or shasum to verify the download"
fi

if [ "$channel" = "stable" ]; then
  tag=$(fetch "https://api.github.com/repos/$REPO/releases/latest" |
    sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
  [ -n "$tag" ] || die "could not resolve the latest release; downloads are at $release_page"
  case "$tag" in
    v*.*.*) ;;
    *) die "latest release returned an unexpected tag '$tag'" ;;
  esac
  case "$tag" in
    *[!0-9A-Za-z._+-]*) die "latest release returned an unsafe tag '$tag'" ;;
  esac
fi

archive="soulseek-rs-$tag-$target.tar.gz"
base_url="https://github.com/$REPO/releases/download/$tag"

tmp=""
staged=""
cleanup() {
  [ -z "$staged" ] || rm -f "$staged" >/dev/null 2>&1 || :
  [ -z "$tmp" ] || rm -rf "$tmp" >/dev/null 2>&1 || :
}
trap cleanup EXIT HUP INT TERM

tmp=$(mktemp -d) || die "could not create a temporary directory"
say "Downloading $archive"
fetch "$base_url/$archive" >"$tmp/$archive" || die "could not download $archive from $release_page"
fetch "$base_url/soulseek-rs-$tag-$target.sha256" >"$tmp/expected.sha256" ||
  die "could not download the sha256 checksum from $release_page"

got=$(sha256 "$tmp/$archive") || die "could not calculate the sha256 for $archive"
got=${got%% *}
want=""
IFS=' ' read -r want _ <"$tmp/expected.sha256" || :
[ -n "$want" ] || die "the sha256 checksum from $release_page was empty"
[ "$got" = "$want" ] || die "sha256 mismatch for $archive: expected $want, got $got"

tar -xzf "$tmp/$archive" -C "$tmp" || die "could not unpack $archive"
[ -f "$tmp/soulseek-rs" ] || die "archive $archive did not contain a soulseek-rs binary"

dir="${SOULSEEK_RS_INSTALL_DIR:-}"
if [ -z "$dir" ]; then
  dir="/usr/local/bin"
  if ! { [ -d "$dir" ] && [ -w "$dir" ]; }; then
    [ -n "${HOME:-}" ] ||
      die "HOME is not set and /usr/local/bin is not writable; set SOULSEEK_RS_INSTALL_DIR"
    dir="$HOME/.local/bin"
  fi
fi
mkdir -p "$dir" || die "could not create install directory '$dir'"
[ -d "$dir" ] || die "install destination '$dir' is not a directory"
[ ! -d "$dir/soulseek-rs" ] || die "install destination '$dir/soulseek-rs' is a directory"

staged=$(mktemp "$dir/.soulseek-rs.XXXXXX") ||
  die "could not create a temporary file in '$dir'"
cp "$tmp/soulseek-rs" "$staged" || die "could not stage the downloaded binary in '$dir'"
chmod 755 "$staged" || die "could not make the downloaded binary executable"
version=$("$staged" --version) ||
  die "the downloaded binary could not run on this machine; the existing install was not changed"
case "$version" in
  soulseek-rs*) ;;
  *) die "the downloaded binary returned unexpected version output '$version'; the existing install was not changed" ;;
esac

mv -f "$staged" "$dir/soulseek-rs" || die "could not replace '$dir/soulseek-rs'"
staged=""

say "Installed $version ($channel) to $dir/soulseek-rs"
case ":${PATH:-}:" in
  *:"$dir":*) ;;
  *) say "note: $dir is not on your PATH; add it with: export PATH=\"$dir:\$PATH\"" ;;
esac
