#!/bin/sh
# Hermetic coverage for public/install.sh. Set INSTALL_TEST_NETWORK=stable to
# add a live stable-release install, or =all after a nightly release exists to
# exercise both public channels.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
script="$here/../public/install.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

failures=0
report() {
  if [ "$1" -eq 0 ]; then
    printf 'ok   %s\n' "$2"
  else
    printf 'FAIL %s\n' "$2"
    failures=$((failures + 1))
  fi
}
contains() { case "$1" in *"$2"*) true ;; *) false ;; esac; }

if command -v sha256sum >/dev/null 2>&1; then
  fixture_sha256() { sha256sum "$1"; }
else
  fixture_sha256() { shasum -a 256 "$1"; }
fi

mkdir -p "$tmp/fixtures/good" "$tmp/fixtures/missing" "$tmp/fixtures/bad-version" \
  "$tmp/fixtures/cannot-run"
printf '#!/bin/sh\nprintf "soulseek-rs 99.0.0-test\\n"\n' >"$tmp/fixtures/good/soulseek-rs"
printf 'no binary here\n' >"$tmp/fixtures/missing/README.md"
printf '#!/bin/sh\nprintf "not-the-client 99.0.0-test\\n"\n' >"$tmp/fixtures/bad-version/soulseek-rs"
printf '#!/bin/sh\nexit 7\n' >"$tmp/fixtures/cannot-run/soulseek-rs"
chmod +x "$tmp/fixtures/good/soulseek-rs" "$tmp/fixtures/bad-version/soulseek-rs" \
  "$tmp/fixtures/cannot-run/soulseek-rs"
tar -czf "$tmp/fixtures/good.tar.gz" -C "$tmp/fixtures/good" soulseek-rs
tar -czf "$tmp/fixtures/missing.tar.gz" -C "$tmp/fixtures/missing" README.md
tar -czf "$tmp/fixtures/bad-version.tar.gz" -C "$tmp/fixtures/bad-version" soulseek-rs
tar -czf "$tmp/fixtures/cannot-run.tar.gz" -C "$tmp/fixtures/cannot-run" soulseek-rs
printf 'this is not a tar archive\n' >"$tmp/fixtures/corrupt.tar.gz"
fixture_sum=$(fixture_sha256 "$tmp/fixtures/good.tar.gz")
fixture_sum=${fixture_sum%% *}
printf '%s  fixture.tar.gz\n' "$fixture_sum" >"$tmp/fixtures/good.sha256"
missing_sum=$(fixture_sha256 "$tmp/fixtures/missing.tar.gz")
missing_sum=${missing_sum%% *}
printf '%s  fixture.tar.gz\n' "$missing_sum" >"$tmp/fixtures/missing.sha256"
bad_version_sum=$(fixture_sha256 "$tmp/fixtures/bad-version.tar.gz")
bad_version_sum=${bad_version_sum%% *}
printf '%s  fixture.tar.gz\n' "$bad_version_sum" >"$tmp/fixtures/bad-version.sha256"
cannot_run_sum=$(fixture_sha256 "$tmp/fixtures/cannot-run.tar.gz")
cannot_run_sum=${cannot_run_sum%% *}
printf '%s  fixture.tar.gz\n' "$cannot_run_sum" >"$tmp/fixtures/cannot-run.sha256"
corrupt_sum=$(fixture_sha256 "$tmp/fixtures/corrupt.tar.gz")
corrupt_sum=${corrupt_sum%% *}
printf '%s  fixture.tar.gz\n' "$corrupt_sum" >"$tmp/fixtures/corrupt.sha256"

# Every fetch is local. The same stub accepts curl's and wget's argument forms,
# records the final URL, and can inject each failure mode the installer handles.
mkdir -p "$tmp/stub"
cat >"$tmp/stub/uname" <<'EOF'
#!/bin/sh
[ -z "${UNAME_LOG:-}" ] || printf '%s\n' "${1:-system}" >>"$UNAME_LOG"
case "${1:-}" in
  -m) printf '%s\n' "$STUB_ARCH" ;;
  *) printf '%s\n' "$STUB_OS" ;;
esac
EOF
cat >"$tmp/stub/fetch" <<'EOF'
#!/bin/sh
url=""
for argument do url=$argument; done
[ -z "${FETCH_LOG:-}" ] || printf '%s\n' "$url" >>"$FETCH_LOG"
case "$url" in
  *api.github.com*)
    [ "${API_MODE:-ok}" = "ok" ] || exit 22
    printf '{"tag_name": "%s"}\n' "${API_TAG:-v12.0.0}"
    ;;
  *.sha256)
    case "${CHECKSUM_MODE:-good}" in
      good) cat "$FIXTURE_DIR/${ARCHIVE_MODE:-good}.sha256" ;;
      bad) printf '%064d  fixture.tar.gz\n' 0 ;;
      empty) : ;;
      fail) exit 22 ;;
      *) exit 64 ;;
    esac
    ;;
  *.tar.gz)
    case "${ARCHIVE_MODE:-good}" in
      good) cat "$FIXTURE_DIR/good.tar.gz" ;;
      missing) cat "$FIXTURE_DIR/missing.tar.gz" ;;
      bad-version) cat "$FIXTURE_DIR/bad-version.tar.gz" ;;
      cannot-run) cat "$FIXTURE_DIR/cannot-run.tar.gz" ;;
      corrupt) cat "$FIXTURE_DIR/corrupt.tar.gz" ;;
      fail) exit 22 ;;
      *) exit 64 ;;
    esac
    ;;
  *) exit 22 ;;
esac
EOF
cat >"$tmp/brew-stub" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >"$BREW_LOG"
[ "${BREW_MODE:-ok}" = ok ] || exit 23
EOF
chmod +x "$tmp/stub/uname" "$tmp/stub/fetch" "$tmp/brew-stub"
ln -s "$tmp/stub/fetch" "$tmp/stub/curl"

base_path="$tmp/stub:/usr/bin:/bin"
fetch_log="$tmp/fetch.log"
uname_log="$tmp/uname.log"
brew_log="$tmp/brew.log"

fixture_install() {
  destination=$1
  shift
  : >"$fetch_log"
  STUB_OS="${STUB_OS:-Darwin}" STUB_ARCH="${STUB_ARCH:-arm64}" \
    FETCH_LOG="$fetch_log" FIXTURE_DIR="$tmp/fixtures" \
    SOULSEEK_RS_INSTALL_DIR="$destination" PATH="$base_path" \
    /bin/sh "$script" "$@"
}

# Argument parsing happens before platform detection, including when piped via
# `sh -s --`. Help must therefore work even on an unsupported machine.
: >"$uname_log"
out=$(STUB_OS=Plan9 STUB_ARCH=mips UNAME_LOG="$uname_log" PATH="$base_path" \
  /bin/sh "$script" --help 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -eq 0 ] && contains "$out" "sh -s -- --nightly" && [ ! -s "$uname_log" ]; then res=0; fi
report "$res" "--help works without probing the machine"

out=$(PATH="$base_path" /bin/sh "$script" --unknown 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "unknown option '--unknown'"; then res=0; fi
report "$res" "unknown options fail with guidance"

out=$(PATH="$base_path" /bin/sh "$script" -- nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "unexpected argument 'nightly'"; then res=0; fi
report "$res" "positional arguments are rejected"

resolved_target() {
  : >"$fetch_log"
  STUB_OS="$1" STUB_ARCH="$2" ARCHIVE_MODE=fail FETCH_LOG="$fetch_log" \
    FIXTURE_DIR="$tmp/fixtures" SOULSEEK_RS_INSTALL_DIR="$tmp/detection" \
    PATH="$base_path" /bin/sh "$script" >/dev/null 2>&1 || true
  sed -n 's#^.*/\(soulseek-rs-v12\.0\.0-[a-z0-9_-]*\.tar\.gz\)$#\1#p' "$fetch_log" | head -n1
}

for spec in \
  "Darwin arm64 aarch64-apple-darwin" \
  "Darwin x86_64 x86_64-apple-darwin" \
  "Linux aarch64 aarch64-unknown-linux-musl" \
  "Linux amd64 x86_64-unknown-linux-musl"; do
  # The record deliberately relies on POSIX field splitting.
  # shellcheck disable=SC2086
  set -- $spec
  res=1
  if [ "$(resolved_target "$1" "$2")" = "soulseek-rs-v12.0.0-$3.tar.gz" ]; then res=0; fi
  report "$res" "$1/$2 resolves to $3"
done

out=$(STUB_OS=MINGW64_NT-10.0 STUB_ARCH=x86_64 PATH="$base_path" \
  /bin/sh "$script" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "pc-windows-msvc" && contains "$out" "releases/tag/nightly"; then res=0; fi
report "$res" "Windows refuses and links to the selected channel"

out=$(STUB_OS=Linux STUB_ARCH=armv7l PATH="$base_path" /bin/sh "$script" 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "unsupported architecture 'armv7l'"; then res=0; fi
report "$res" "unsupported architectures fail before downloading"

out=$(STUB_OS=Plan9 STUB_ARCH=x86_64 PATH="$base_path" /bin/sh "$script" 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "unsupported OS 'Plan9'"; then res=0; fi
report "$res" "unsupported operating systems fail before downloading"

# Stable uses Homebrew when available. Nightly must bypass it because the tap
# intentionally tracks stable releases only.
mkdir -p "$tmp/with-brew"
ln -s "$tmp/stub/uname" "$tmp/with-brew/uname"
ln -s "$tmp/brew-stub" "$tmp/with-brew/brew"
ln -s "$tmp/stub/fetch" "$tmp/with-brew/curl"
: >"$fetch_log"
BREW_LOG="$brew_log" STUB_OS=Darwin STUB_ARCH=arm64 FETCH_LOG="$fetch_log" \
  PATH="$tmp/with-brew:/usr/bin:/bin" /bin/sh "$script" >/dev/null 2>&1
res=1
if [ "$(cat "$brew_log")" = "install michel/tap/soulseek-rs" ] && [ ! -s "$fetch_log" ]; then res=0; fi
report "$res" "stable uses Homebrew without touching GitHub"

out=$(BREW_MODE=fail BREW_LOG="$brew_log" STUB_OS=Darwin STUB_ARCH=arm64 \
  PATH="$tmp/with-brew:/usr/bin:/bin" /bin/sh "$script" 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -eq 23 ]; then res=0; fi
report "$res" "Homebrew failures propagate to the caller"

nightly_dir="$tmp/nightly install/bin"
: >"$brew_log"
out=$(BREW_LOG="$brew_log" STUB_OS=Darwin STUB_ARCH=arm64 \
  FETCH_LOG="$fetch_log" FIXTURE_DIR="$tmp/fixtures" \
  SOULSEEK_RS_INSTALL_DIR="$nightly_dir" PATH="$tmp/with-brew:/usr/bin:/bin" \
  /bin/sh "$script" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -eq 0 ] && [ -x "$nightly_dir/soulseek-rs" ] &&
  contains "$out" "(nightly)" && [ ! -s "$brew_log" ] &&
  ! grep -q 'api.github.com' "$fetch_log" &&
  grep -q '/nightly/soulseek-rs-nightly-aarch64-apple-darwin.tar.gz$' "$fetch_log"; then res=0; fi
report "$res" "--nightly bypasses Homebrew and the stable-release API"

custom_stable_dir="$tmp/custom stable/bin"
: >"$brew_log"
out=$(BREW_LOG="$brew_log" STUB_OS=Darwin STUB_ARCH=arm64 \
  FETCH_LOG="$fetch_log" FIXTURE_DIR="$tmp/fixtures" \
  SOULSEEK_RS_INSTALL_DIR="$custom_stable_dir" PATH="$tmp/with-brew:/usr/bin:/bin" \
  /bin/sh "$script" 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -eq 0 ] && [ -x "$custom_stable_dir/soulseek-rs" ] &&
  [ ! -s "$brew_log" ] && grep -q 'api.github.com' "$fetch_log"; then res=0; fi
report "$res" "custom install directory overrides Homebrew"

stable_dir="$tmp/stable/bin"
out=$(fixture_install "$stable_dir" 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -eq 0 ] && [ -x "$stable_dir/soulseek-rs" ] &&
  [ "$("$stable_dir/soulseek-rs" --version)" = "soulseek-rs 99.0.0-test" ] &&
  contains "$out" "(stable)" &&
  grep -q '/v12.0.0/soulseek-rs-v12.0.0-aarch64-apple-darwin.tar.gz$' "$fetch_log"; then res=0; fi
report "$res" "verified stable archive installs and runs"

res=1
if contains "$out" "$stable_dir is not on your PATH"; then res=0; fi
report "$res" "a destination off PATH gets a usable PATH hint"

# A minimal PATH with wget but no curl proves the fallback instead of merely
# exercising a wget-shaped stub while curl still wins command discovery.
mkdir -p "$tmp/wget-path"
for tool in cat chmod cp gzip head mkdir mktemp mv rm sed shasum sha256sum tar; do
  tool_path=$(command -v "$tool" 2>/dev/null || true)
  [ -z "$tool_path" ] || ln -s "$tool_path" "$tmp/wget-path/$tool"
done
ln -s "$tmp/stub/uname" "$tmp/wget-path/uname"
ln -s "$tmp/stub/fetch" "$tmp/wget-path/wget"
wget_dir="$tmp/wget/bin"
: >"$fetch_log"
out=$(STUB_OS=Darwin STUB_ARCH=arm64 FETCH_LOG="$fetch_log" \
  FIXTURE_DIR="$tmp/fixtures" SOULSEEK_RS_INSTALL_DIR="$wget_dir" \
  PATH="$tmp/wget-path" /bin/sh "$script" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -eq 0 ] && [ -x "$wget_dir/soulseek-rs" ]; then res=0; fi
report "$res" "wget-only installation works"

# Failures before replacement must preserve an existing install and leave no
# fixed-name staging file behind.
preserve_dir="$tmp/preserve/bin"
mkdir -p "$preserve_dir"
printf '#!/bin/sh\nprintf "soulseek-rs old\\n"\n' >"$preserve_dir/soulseek-rs"
chmod +x "$preserve_dir/soulseek-rs"
out=$(CHECKSUM_MODE=bad fixture_install "$preserve_dir" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "sha256 mismatch" &&
  [ "$("$preserve_dir/soulseek-rs")" = "soulseek-rs old" ]; then res=0; fi
report "$res" "checksum mismatch preserves the installed binary"

out=$(CHECKSUM_MODE=empty fixture_install "$preserve_dir" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "checksum" && contains "$out" "empty"; then res=0; fi
report "$res" "empty checksum is rejected"

out=$(ARCHIVE_MODE=missing fixture_install "$preserve_dir" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "did not contain a soulseek-rs binary" &&
  [ "$("$preserve_dir/soulseek-rs")" = "soulseek-rs old" ]; then res=0; fi
report "$res" "archive without the binary preserves the installed binary"

out=$(ARCHIVE_MODE=bad-version fixture_install "$preserve_dir" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "unexpected version output" &&
  [ "$("$preserve_dir/soulseek-rs")" = "soulseek-rs old" ]; then res=0; fi
report "$res" "wrong executable preserves the installed binary"

out=$(ARCHIVE_MODE=cannot-run fixture_install "$preserve_dir" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "could not run on this machine" &&
  [ "$("$preserve_dir/soulseek-rs")" = "soulseek-rs old" ]; then res=0; fi
report "$res" "non-running executable preserves the installed binary"

out=$(ARCHIVE_MODE=corrupt fixture_install "$preserve_dir" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "could not unpack" &&
  [ "$("$preserve_dir/soulseek-rs")" = "soulseek-rs old" ]; then res=0; fi
report "$res" "corrupt archive preserves the installed binary"

out=$(ARCHIVE_MODE=fail fixture_install "$preserve_dir" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "could not download soulseek-rs-nightly" &&
  [ "$("$preserve_dir/soulseek-rs")" = "soulseek-rs old" ]; then res=0; fi
report "$res" "archive download failure preserves the installed binary"

out=$(CHECKSUM_MODE=fail fixture_install "$preserve_dir" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "could not download the sha256 checksum" &&
  [ "$("$preserve_dir/soulseek-rs")" = "soulseek-rs old" ]; then res=0; fi
report "$res" "checksum download failure preserves the installed binary"

directory_target="$tmp/directory-target/bin"
mkdir -p "$directory_target/soulseek-rs"
out=$(fixture_install "$directory_target" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "is a directory" &&
  [ ! -e "$directory_target/soulseek-rs/.soulseek-rs.new" ]; then res=0; fi
report "$res" "a directory at the binary path is never written into"

out=$(API_MODE=fail fixture_install "$tmp/api-failure" 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "could not resolve the latest release"; then res=0; fi
report "$res" "stable API failure is explained"

: >"$fetch_log"
out=$(API_TAG='../nightly' fixture_install "$tmp/unsafe-tag" 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "unexpected tag" &&
  [ "$(wc -l <"$fetch_log" | tr -d ' ')" = 1 ]; then res=0; fi
report "$res" "unexpected stable tag is rejected before asset download"

mkdir -p "$tmp/no-fetch"
ln -s "$tmp/stub/uname" "$tmp/no-fetch/uname"
out=$(STUB_OS=Darwin STUB_ARCH=arm64 PATH="$tmp/no-fetch" /bin/sh "$script" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "need curl or wget"; then res=0; fi
report "$res" "missing downloader is reported"

mkdir -p "$tmp/no-tar"
ln -s "$tmp/stub/uname" "$tmp/no-tar/uname"
ln -s "$tmp/stub/fetch" "$tmp/no-tar/curl"
out=$(STUB_OS=Darwin STUB_ARCH=arm64 PATH="$tmp/no-tar" /bin/sh "$script" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "need tar"; then res=0; fi
report "$res" "missing tar is reported before downloading"

mkdir -p "$tmp/no-sha"
ln -s "$tmp/stub/uname" "$tmp/no-sha/uname"
ln -s "$tmp/stub/fetch" "$tmp/no-sha/curl"
tar_path=$(command -v tar)
ln -s "$tar_path" "$tmp/no-sha/tar"
out=$(STUB_OS=Darwin STUB_ARCH=arm64 PATH="$tmp/no-sha" /bin/sh "$script" --nightly 2>&1) && rc=0 || rc=$?
res=1
if [ "$rc" -ne 0 ] && contains "$out" "need sha256sum or shasum"; then res=0; fi
report "$res" "missing checksum tool is reported before downloading"

# Two callers can update the same path without sharing a predictable staging
# name. Either can win; the result must run and no staging file may remain.
concurrent_dir="$tmp/concurrent/bin"
fixture_install "$concurrent_dir" --nightly >/dev/null 2>&1 & first=$!
fixture_install "$concurrent_dir" --nightly >/dev/null 2>&1 & second=$!
first_rc=0
second_rc=0
wait "$first" || first_rc=$?
wait "$second" || second_rc=$?
set -- "$concurrent_dir"/.soulseek-rs.*
res=1
if [ "$first_rc" -eq 0 ] && [ "$second_rc" -eq 0 ] &&
  [ "$("$concurrent_dir/soulseek-rs" --version)" = "soulseek-rs 99.0.0-test" ] &&
  [ ! -e "$1" ]; then res=0; fi
report "$res" "concurrent installers leave one complete binary"

case "${INSTALL_TEST_NETWORK:-}" in
  "") ;;
  stable | all)
    network_path="/usr/bin:/bin"
    live_stable="$tmp/live-stable"
    out=$(SOULSEEK_RS_INSTALL_DIR="$live_stable" PATH="$network_path" \
      /bin/sh "$script" 2>&1) && rc=0 || rc=$?
    res=1
    if [ "$rc" -eq 0 ] && "$live_stable/soulseek-rs" --version >/dev/null 2>&1; then res=0; fi
    report "$res" "live stable release installs and runs"

    if [ "${INSTALL_TEST_NETWORK:-}" = all ]; then
      live_nightly="$tmp/live-nightly"
      out=$(SOULSEEK_RS_INSTALL_DIR="$live_nightly" PATH="$network_path" \
        /bin/sh "$script" --nightly 2>&1) && rc=0 || rc=$?
      res=1
      if [ "$rc" -eq 0 ] && "$live_nightly/soulseek-rs" --version >/dev/null 2>&1; then res=0; fi
      report "$res" "live nightly release installs and runs"
    fi
    ;;
  *)
    printf 'FAIL INSTALL_TEST_NETWORK must be empty, stable, or all\n'
    failures=$((failures + 1))
    ;;
esac

[ "$failures" -eq 0 ] || {
  printf '%s test(s) failed\n' "$failures"
  exit 1
}
printf 'all tests passed\n'
