#!/usr/bin/env bash
# Build and test every package. Runs all of them even if one fails; exits non-zero if any did.
# Usage: scripts/test.sh
set -u
root="$(cd "$(dirname "$0")/.." && pwd)"
failed=()

run() {
  local name="$1"; shift
  echo "==> $name"
  if ! (cd "$root/$name" && "$@"); then
    failed+=("$name")
  fi
}

run executor-macos swift test
run stand-in npm test
run harness npm test
run schema npm test

echo
if [ "${#failed[@]}" -gt 0 ]; then
  echo "FAILED: ${failed[*]}"
  exit 1
fi
echo "All packages passed."
