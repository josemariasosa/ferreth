#!/usr/bin/env bash
set -euo pipefail
evidence=$(cd "$(dirname "$0")" && pwd -P)
repo=${1:?Pass the absolute Ferreth repository path}
out=${2:?Pass a fresh absolute evidence output path outside the repository}
mkdir "$out"
export CARGO_TARGET_DIR="$out/target"

cd "$repo"
git status --porcelain=v1 --branch --untracked-files=all > "$out/git.before"
git rev-parse HEAD > "$out/revision"
rustc -Vv > "$out/rustc.version"
cargo -V > "$out/cargo.version"
cargo tree --frozen -p ferreth-compiler --depth 1 > "$out/compiler.tree"
cargo tree --frozen -p ferreth-conformance --depth 1 > "$out/conformance.tree"
git hash-object Cargo.lock > "$out/lock.blob"
command -v dtc > "$out/dtc.path" || test "$?" -eq 1
set +e
cargo run --frozen -q -p ferreth-conformance -- bench --op ALL --cases 1000 --seed 42 --format json > "$out/core.stdout" 2> "$out/core.stderr"
printf '%s\n' "$?" > "$out/core.exit"
cargo run --frozen -q -p ferreth-conformance -- bench guest --format json > "$out/guest.stdout" 2> "$out/guest.stderr"
printf '%s\n' "$?" > "$out/guest.exit"
set -e

find_rlib() {
    local matches=("$CARGO_TARGET_DIR/debug/deps/lib$1"-*.rlib)
    test "${#matches[@]}" -eq 1
    test -f "${matches[0]}"
    printf '%s\n' "${matches[0]}"
}
rustc --edition=2024 "$evidence/observe.rs" \
    -L "dependency=$CARGO_TARGET_DIR/debug/deps" \
    --extern "ferreth_compiler=$(find_rlib ferreth_compiler)" \
    --extern "alloy_primitives=$(find_rlib alloy_primitives)" \
    --extern "asm_rs=$(find_rlib asm_rs)" \
    -o "$out/observe" > "$out/observer-build.stdout" 2> "$out/observer-build.stderr"
cd "$out"
./observe specimens > observer.stdout 2> observer.stderr
cd "$repo"
git status --porcelain=v1 --branch --untracked-files=all > "$out/git.after"
git rev-parse HEAD > "$out/revision.after"
git diff --exit-code HEAD --
git diff --cached --exit-code
cmp "$out/git.before" "$out/git.after"
cmp "$out/revision" "$out/revision.after"
python3 "$evidence/collect.py" "$out"
