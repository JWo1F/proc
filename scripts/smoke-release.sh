#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT_DIR/target/release/proc"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/proc-smoke.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

run_expect_success() {
  local name="$1"
  shift

  set +e
  "$@" >"$TMP_DIR/$name.out" 2>"$TMP_DIR/$name.err"
  local code=$?
  set -e

  if [[ $code -ne 0 ]]; then
    echo "FAIL [$name]: expected success, got exit code $code"
    cat "$TMP_DIR/$name.out" "$TMP_DIR/$name.err"
    exit 1
  fi

  echo "PASS [$name]"
}

run_expect_failure() {
  local name="$1"
  shift

  set +e
  "$@" >"$TMP_DIR/$name.out" 2>"$TMP_DIR/$name.err"
  local code=$?
  set -e

  if [[ $code -eq 0 ]]; then
    echo "FAIL [$name]: expected non-zero exit code"
    cat "$TMP_DIR/$name.out" "$TMP_DIR/$name.err"
    exit 1
  fi

  echo "PASS [$name]"
}

cd "$ROOT_DIR"
echo "Building release binary..."
cargo build --release

INVALID_FILE="$TMP_DIR/invalid.procfile"
COMMENT_FILE="$TMP_DIR/comment.procfile"
EXCLUDE_FILE="$TMP_DIR/exclude.procfile"
MISSING_FILE="$TMP_DIR/missing.procfile"

cat >"$INVALID_FILE" <<'EOF'
this is invalid
EOF

cat >"$COMMENT_FILE" <<'EOF'
  # indented comment
web: echo hi
EOF

cat >"$EXCLUDE_FILE" <<'EOF'
web: echo hi
EOF

run_expect_failure "missing-file" "$BIN" -f "$MISSING_FILE"
run_expect_failure "parse-error" "$BIN" -f "$INVALID_FILE"
run_expect_success "indented-comment" "$BIN" -f "$COMMENT_FILE"
run_expect_failure "all-excluded" "$BIN" -f "$EXCLUDE_FILE" -x web

echo "Smoke tests passed."
