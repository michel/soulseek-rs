#!/bin/sh
# Exercises public/install.sh: target detection, the brew fast path, checksum
# verification, and a real install into a throwaway prefix. Network tests hit
# GitHub; run from a machine that can reach it.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
script="$here/../public/install.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

failures=0
report() {
  if [ "$1" = 0 ]; then
    printf 'ok   %s\n' "$2"
  else
    printf 'FAIL %s\n' "$2"
    failures=$((failures + 1))
  fi
}
contains() { case "$1" in *"$2"*) true ;; *) false ;; esac; }

# Stub uname reads STUB_OS / STUB_ARCH; stub curl logs every URL it is asked
# for, answers the release-API call with a canned tag, and refuses the rest so
# a detection test stops before downloading anything.
mkdir -p "$tmp/stub"
cat >"$tmp/stub/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
  -m) echo "$STUB_ARCH" ;;
  *) echo "$STUB_OS" ;;
esac
EOF
cat >"$tmp/stub/curl" <<EOF
#!/bin/sh
for a; do url=\$a; done
echo "\$url" >>"$tmp/curl.log"
case "\$url" in
  *api.github.com*) echo '{"tag_name": "v12.0.0"}' ;;
  *) exit 22 ;;
esac
EOF
chmod +x "$tmp/stub/uname" "$tmp/stub/curl"
base_path="/usr/bin:/bin"

resolved_target() {
  : >"$tmp/curl.log"
  STUB_OS="$1" STUB_ARCH="$2" PATH="$tmp/stub:$base_path" sh "$script" >/dev/null 2>&1 || true
  grep -o 'soulseek-rs-v12\.0\.0-[a-z0-9_-]*\.tar\.gz' "$tmp/curl.log" | head -n1
}

for spec in \
  "Darwin arm64 aarch64-apple-darwin" \
  "Darwin x86_64 x86_64-apple-darwin" \
  "Linux aarch64 aarch64-unknown-linux-musl" \
  "Linux amd64 x86_64-unknown-linux-musl"; do
  # spec is three space-separated fields, so the splitting is the point
  # shellcheck disable=SC2086
  set -- $spec
  res=1
  if [ "$(resolved_target "$1" "$2")" = "soulseek-rs-v12.0.0-$3.tar.gz" ]; then res=0; fi
  report $res "$1/$2 resolves to $3"
done

out=$(STUB_OS="MINGW64_NT-10.0" STUB_ARCH=x86_64 PATH="$tmp/stub:$base_path" sh "$script" 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" != 0 ] && contains "$out" "pc-windows-msvc"; then res=0; fi
report $res "Windows refuses and points at the msvc zip"

out=$(STUB_OS=Linux STUB_ARCH=armv7l PATH="$tmp/stub:$base_path" sh "$script" 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" != 0 ] && contains "$out" "unsupported architecture"; then res=0; fi
report $res "armv7l refuses as unsupported"

# Brew fast path: a stub brew must be exec'd with the tap formula, and no URL
# may be fetched.
cat >"$tmp/stub/brew" <<EOF
#!/bin/sh
echo "\$*" >"$tmp/brew.log"
EOF
chmod +x "$tmp/stub/brew"
: >"$tmp/curl.log"
STUB_OS=Darwin STUB_ARCH=arm64 PATH="$tmp/stub:$base_path" sh "$script" >/dev/null 2>&1
res=1
if [ "$(cat "$tmp/brew.log")" = "install michel/tap/soulseek-rs" ] && [ ! -s "$tmp/curl.log" ]; then res=0; fi
report $res "brew on PATH wins and skips the download"
rm -f "$tmp/stub/brew"

# Offline happy path: stub curl serves a locally built archive whose sha256
# matches, and the installed fake binary must answer --version.
mkdir -p "$tmp/payload" "$tmp/bin"
printf '#!/bin/sh\necho "soulseek-rs 99.0.0-test"\n' >"$tmp/payload/soulseek-rs"
chmod +x "$tmp/payload/soulseek-rs"
tar -czf "$tmp/payload.tar.gz" -C "$tmp/payload" soulseek-rs
sum=$(shasum -a 256 "$tmp/payload.tar.gz" | cut -d' ' -f1)
cat >"$tmp/stub/curl" <<EOF
#!/bin/sh
for a; do url=\$a; done
echo "\$url" >>"$tmp/curl.log"
case "\$url" in
  *api.github.com*) echo '{"tag_name": "v12.0.0"}' ;;
  *.tar.gz) cat "$tmp/payload.tar.gz" ;;
  *.sha256) echo "$sum *payload" ;;
  *) exit 22 ;;
esac
EOF
out=$(STUB_OS=Darwin STUB_ARCH=arm64 SOULSEEK_RS_INSTALL_DIR="$tmp/bin" \
  PATH="$tmp/stub:$base_path" sh "$script" 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" = 0 ] && [ -x "$tmp/bin/soulseek-rs" ] &&
  contains "$out" "Installed soulseek-rs 99.0.0-test"; then res=0; fi
report $res "verified archive installs and reports the version"
res=1
if contains "$out" "not on your PATH"; then res=0; fi
report $res "install dir off the PATH gets the PATH note"

# Tampered archive: same stub but a wrong checksum must abort before install.
rm -f "$tmp/bin/soulseek-rs"
sed "s/$sum/0000000000000000000000000000000000000000000000000000000000000000/" \
  "$tmp/stub/curl" >"$tmp/stub/curl.bad" && mv "$tmp/stub/curl.bad" "$tmp/stub/curl"
chmod +x "$tmp/stub/curl"
out=$(STUB_OS=Darwin STUB_ARCH=arm64 SOULSEEK_RS_INSTALL_DIR="$tmp/bin" \
  PATH="$tmp/stub:$base_path" sh "$script" 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" != 0 ] && [ ! -e "$tmp/bin/soulseek-rs" ] &&
  contains "$out" "sha256 mismatch"; then res=0; fi
report $res "sha256 mismatch aborts without installing"

# Real network installs: once over curl, once over wget (a symlink farm of
# /usr/bin and /bin minus curl and brew forces the wget code path).
out=$(SOULSEEK_RS_INSTALL_DIR="$tmp/real-curl" PATH="$base_path" sh "$script" 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" = 0 ] && "$tmp/real-curl/soulseek-rs" --version >/dev/null 2>&1; then res=0; fi
report $res "real release installs via curl and runs"

if wget=$(command -v wget); then
  mkdir -p "$tmp/farm"
  for d in /usr/bin /bin; do
    for f in "$d"/*; do ln -sf "$f" "$tmp/farm/${f##*/}"; done
  done
  rm -f "$tmp/farm/curl" "$tmp/farm/brew"
  ln -sf "$wget" "$tmp/farm/wget"
  out=$(SOULSEEK_RS_INSTALL_DIR="$tmp/real-wget" PATH="$tmp/farm" sh "$script" 2>&1) && rc=0 || rc=$?
  res=1
  if [ "$rc" = 0 ] && "$tmp/real-wget/soulseek-rs" --version >/dev/null 2>&1; then res=0; fi
  report $res "real release installs via wget and runs"
else
  echo "skip real release installs via wget (no wget on this machine)"
fi

[ "$failures" = 0 ] || {
  echo "$failures test(s) failed"
  exit 1
}
echo "all tests passed"
