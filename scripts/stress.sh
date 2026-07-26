#!/usr/bin/env bash
# Run the concurrency stress benchmark against a local soulfind.
#
#   scripts/stress.sh                 # run with the default load profile
#   scripts/stress.sh --save          # store the run as the new baseline if it wins
#   STRESS_PEERS=128 scripts/stress.sh
#
# Knobs: STRESS_PEERS STRESS_SEARCHES STRESS_DOWNLOADS STRESS_LEECHERS
#        STRESS_BROWSES STRESS_FILE_KB STRESS_TIMEOUT
set -euo pipefail

cd "$(dirname "$0")/.."

# soulfind is a D binary linked against Homebrew's SQLite.
export SOULFIND_BIN="${SOULFIND_BIN:-$HOME/src/soulseek_download/soulfind/bin/soulfind}"
export DYLD_LIBRARY_PATH="${DYLD_LIBRARY_PATH:-/opt/homebrew/opt/sqlite/lib}"

if [ ! -x "$SOULFIND_BIN" ]; then
  echo "stress: no soulfind at $SOULFIND_BIN — set SOULFIND_BIN" >&2
  exit 2
fi

# Stale databases from a killed run make the next login behave oddly.
rm -f "${TMPDIR:-/tmp}"/soulfind-stress-*.db

# Every mock peer holds sockets open; the default 256 descriptors is nowhere near
# enough for a swarm.
ulimit -n 20000 2>/dev/null || true

for arg in "$@"; do
  case "$arg" in
    --save) export STRESS_SAVE=1 ;;
    *) echo "stress: unknown argument $arg" >&2; exit 2 ;;
  esac
done

exec cargo run --release --quiet -p soulseek-rs-lib --example stress
