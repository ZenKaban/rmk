#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

export RUST_MIN_STACK="${RUST_MIN_STACK:-33554432}"

if [[ -z "${BINDGEN_EXTRA_CLANG_ARGS:-}" ]]; then
    clang_args=(-ffreestanding --target=arm-none-eabi)

    if command -v arm-none-eabi-gcc >/dev/null 2>&1; then
        sysroot="$(arm-none-eabi-gcc -print-sysroot 2>/dev/null || true)"
        gcc_include="$(arm-none-eabi-gcc -print-file-name=include 2>/dev/null || true)"

        if [[ -n "$sysroot" && -d "$sysroot" ]]; then
            clang_args+=("--sysroot=$sysroot")
            if [[ -d "$sysroot/include" ]]; then
                clang_args+=("-I$sysroot/include")
            fi
        fi

        if [[ -n "$gcc_include" && -d "$gcc_include" ]]; then
            clang_args+=("-I$gcc_include")
        fi
    else
        xpack_root="${XPACK_ARM_GCC_ROOT:-}"
        if [[ -z "$xpack_root" ]]; then
            for candidate in "$HOME"/.local/xPacks/@xpack-dev-tools/arm-none-eabi-gcc/*/.content; do
                if [[ -d "$candidate/arm-none-eabi/include" ]]; then
                    xpack_root="$candidate"
                fi
            done
        fi

        if [[ -n "$xpack_root" && -d "$xpack_root" ]]; then
            clang_args+=("-I$xpack_root/arm-none-eabi/include")
            gcc_include="$(
                find "$xpack_root/lib/gcc/arm-none-eabi" \
                    -mindepth 2 \
                    -maxdepth 2 \
                    -type d \
                    -name include \
                    2>/dev/null \
                    | sort -V \
                    | tail -1
            )"
            if [[ -n "$gcc_include" ]]; then
                clang_args+=("-I$gcc_include")
            fi
        fi
    fi

    host_gcc_include="$(gcc -print-file-name=include 2>/dev/null || true)"
    if [[ -n "$host_gcc_include" && -d "$host_gcc_include" ]]; then
        clang_args+=("-I$host_gcc_include")
    fi

    export BINDGEN_EXTRA_CLANG_ARGS="${clang_args[*]}"
fi

run() {
    local dir="$1"
    shift
    echo
    echo "==> $dir: $*"
    (
        cd "$repo_root/$dir"
        "$@"
    )
}

build_split() {
    local keyboard="$1"
    local bins=(--bin central --bin peripheral)
    if grep -q 'name = "hardreset"' "$repo_root/keyboards/$keyboard/Cargo.toml"; then
        bins+=(--bin hardreset)
    fi
    run "keyboards/$keyboard" cargo build --release "${bins[@]}"
}

build_qube() {
    local keyboard="$1"
    run "keyboards/$keyboard" env CARGO_TARGET_DIR=target/qube cargo build --release --bin qube --features qube
    run "keyboards/$keyboard" env CARGO_TARGET_DIR=target/halves cargo build --release --bin left --bin right
}

echo "Using BINDGEN_EXTRA_CLANG_ARGS=$BINDGEN_EXTRA_CLANG_ARGS"

build_split k04
build_split k04_mini
build_split k04_micro
build_split op36
build_split k03
build_split imperial44
build_split velvet
build_split velvet_ui
run "keyboards/trackball_v30" cargo build --release --bin keyboard
run "keyboards/trackball_v31" cargo build --release --bin keyboard
run "keyboards/trackball_royale" cargo build --release --bin keyboard
build_qube k04_qube
build_qube op36_qube

echo
echo "Root RMK build matrix OK"
