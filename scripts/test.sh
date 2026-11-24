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
run shell/ui node --test

# The shell's tests are headless (no window, no tray). They ping the real executor, so build it first.
# Needs Rust (rustup) on PATH; the macOS CI runner has it.
# The two real-keychain tests are #[ignore]d so CI never touches the login keychain. Locally:
#   cd shell/src-tauri && cargo test --test store -- --ignored
echo "==> shell (cargo test)"
if ! command -v cargo >/dev/null; then
  echo "cargo not found: install Rust (brew install rustup && rustup-init -y)"
  failed+=("shell")
elif ! (cd "$root/executor-macos" && swift build) \
  || ! (cd "$root/shell/src-tauri" && EXECUTOR_BIN="$root/executor-macos/.build/debug/executor-macos" cargo test); then
  failed+=("shell")
fi

echo
if [ "${#failed[@]}" -gt 0 ]; then
  echo "FAILED: ${failed[*]}"
  exit 1
fi
echo "All packages passed."
