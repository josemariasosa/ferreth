#!/usr/bin/env bash
set -euo pipefail
repo=${1:?Pass the absolute Ferreth checkout path}
out=${2:?Pass a fresh absolute test evidence directory}
mkdir "$out"
snapshot="$out/checkout"
mkdir "$snapshot"
rel=crates/ferreth-compiler/tests/rv64_push1_program.rs
git -C "$repo" archive HEAD | tar -xf - -C "$snapshot"
cp "$repo/$rel" "$snapshot/$rel"
cmp "$repo/$rel" "$snapshot/$rel"
export CARGO_TARGET_DIR="$out/target"

capture() {
    local name=$1 status
    shift
    printf '%q ' "$@" > "$out/$name.command"
    printf '\n' >> "$out/$name.command"
    if "$@" > "$out/$name.stdout" 2> "$out/$name.stderr"; then
        status=0
    else
        status=$?
    fi
    printf '%s\n' "$status" > "$out/$name.exit"
}

cd "$snapshot"
for test in \
    repeated_push1_46_eof_completes_normally \
    repeated_push1_46_stop_completes_normally \
    repeated_push1_47_eof_completes_normally \
    repeated_push1_47_stop_completes_normally \
    repeated_push1_47_eof_full_stack_publishes_overflow \
    repeated_push1_47_stop_full_stack_publishes_overflow; do
    capture "$test" cargo test --frozen -p ferreth-compiler --test rv64_push1_program "$test" -- --exact --test-threads=1
done
capture workspace cargo test --frozen --workspace -- --test-threads=1
capture workspace-complete cargo test --frozen --workspace --no-fail-fast -- --test-threads=1
capture fmt cargo fmt --all -- --check
cmp "$repo/$rel" "$snapshot/$rel"
