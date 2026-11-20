#!/usr/bin/env bash
set -euo pipefail
repo=${1:?Pass the absolute Ferreth checkout path}
out=${2:?Pass the absolute evidence output directory}
evidence=$(cd "$(dirname "$0")" && pwd -P)
mkdir -p "$out"
export CARGO_TARGET_DIR="$out/target"
cd "$repo"

capture() {
    local name=$1 status
    shift
    if "$@" > "$out/$name.stdout" 2> "$out/$name.stderr"; then
        status=0
    else
        status=$?
    fi
    printf '%s\n' "$status" > "$out/$name.exit"
    return "$status"
}
find_rlib() {
    local matches=("$CARGO_TARGET_DIR/debug/deps/lib$1"-*.rlib)
    test "${#matches[@]}" -eq 1
    test -f "${matches[0]}"
    printf '%s\n' "${matches[0]}"
}

capture build cargo build --frozen -p ferreth-compiler -p revm-frame
capture extract python3 "$evidence/extract.py" "$repo" "$out/encoders.rs"
cp "$evidence/probe.rs" "$out/boundary.rs"
capture probe-build rustc --edition=2024 "$out/boundary.rs" \
    -L "dependency=$CARGO_TARGET_DIR/debug/deps" \
    --extern "ferreth_compiler=$(find_rlib ferreth_compiler)" \
    --extern "asm_rs=$(find_rlib asm_rs)" -o "$out/probe"
cd "$out"
capture probe ./probe

cd "$repo"
eels_python="${EELS_PYTHON:-$repo/research/eels-cancun/.venv/bin/python}"
capture eels-version "$eels_python" --version
capture eels-pin "$eels_python" -c 'from importlib.metadata import distribution; import json; d = json.loads(distribution("ethereum-execution").read_text("direct_url.json")); print(d); assert d["vcs_info"]["commit_id"] == "d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3"'
code=$(python3 -c 'print("6002" * 47)')
for end in eof stop; do
    bytecode="$code"
    if [[ "$end" == stop ]]; then bytecode="${code}00"; fi
    capture "eels-$end" env PYTHONDONTWRITEBYTECODE=1 "$eels_python" \
        "$repo/research/eels-cancun/run_frame.py" --code "$bytecode" --gas 141
    capture "revm-$end" "$CARGO_TARGET_DIR/debug/revm-frame" --code "$bytecode" --gas 141
done
