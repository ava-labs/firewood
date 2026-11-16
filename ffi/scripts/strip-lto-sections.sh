#!/usr/bin/env bash
# Strip LLVM bitcode from static libs, then fail if any section cmd/go rejects
# remains (go1.26.11+/go1.27.4+, CVE-2026-97038; see isDisallowedLTOSection in
# cmd/go/internal/work/security.go). rustup's prebuilt compiler_builtins
# embeds the bitcode. Stripping cannot break a link: without bitcode, even a
# -flto link falls back to the native code in each object.
# Needs the llvm-tools rustup component. Usage: strip-lto-sections.sh LIB...
set -euo pipefail

bin="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin"
for lib in "$@"; do
    "$bin/llvm-objcopy" --remove-section=.llvmbc --remove-section=.llvmcmd \
        --remove-section=__LLVM,__bitcode --remove-section=__LLVM,__cmdline "$lib"
    sections=$("$bin/llvm-readobj" --sections "$lib")
    if grep -E \
        'Segment: __(LLVM|GNU_LTO)|Name: (\.gnu\.(debug)?lto_|\.llvm\.lto|\.llvmbc |\.llvmcmd )|Type: SHT_LLVM_LTO' <<<"$sections"; then
        echo "$lib: disallowed LTO sections remain" >&2
        exit 1
    fi
done
