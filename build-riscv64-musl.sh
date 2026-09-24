#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
TARGET_TRIPLE="riscv64gc-unknown-linux-musl"
TARGET_DIR_NAME="riscv64gc-unknown-linux-musl"
CROSS_LINKER="riscv64-linux-musl-gcc"
OUTPUT_ROOT="$SCRIPT_DIR/dist"
STAGING_DIR=""
BACKUP_DIR=""

usage() {
    printf '%s\n' \
        'Usage:' \
        '  ./build-riscv64-musl.sh [--output-dir DIR]' \
        '  ./build-riscv64-musl.sh --help' \
        '' \
        "Target: $TARGET_TRIPLE" \
        'Requires: bash, cargo, rustup, riscv64-linux-musl-gcc, tar, sha256sum' \
        'The target binary is cross-compiled but never executed by this script.'
}

argument_error() {
    printf 'error: %s\n' "$1" >&2
    usage >&2
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --help|-h)
            usage
            exit 0
            ;;
        --output-dir)
            if [[ $# -lt 2 || "$2" == -* ]]; then
                argument_error "--output-dir requires a directory"
            fi
            OUTPUT_ROOT="$2"
            shift 2
            ;;
        *)
            argument_error "unknown option: $1"
            ;;
    esac
done

if [[ -z "$OUTPUT_ROOT" ]]; then
    argument_error "--output-dir must not be empty"
fi

missing_commands=()
for command_name in bash cargo rustup "$CROSS_LINKER" tar sha256sum; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        missing_commands+=("$command_name")
    fi
done

if [[ ${#missing_commands[@]} -gt 0 ]]; then
    printf 'error: missing required commands: %s\n' "${missing_commands[*]}" >&2
    exit 1
fi

target_installed=0
while IFS= read -r installed_target; do
    if [[ "$installed_target" == "$TARGET_TRIPLE" ]]; then
        target_installed=1
        break
    fi
done < <(rustup target list --installed)

if [[ $target_installed -ne 1 ]]; then
    printf 'error: missing required Rust target: %s\n' "$TARGET_TRIPLE" >&2
    printf 'install it with: rustup target add %s\n' "$TARGET_TRIPLE" >&2
    exit 1
fi

if [[ "$OUTPUT_ROOT" != /* ]]; then
    OUTPUT_ROOT="$PWD/$OUTPUT_ROOT"
fi

mkdir -p -- "$OUTPUT_ROOT"
OUTPUT_ROOT=$(CDPATH= cd -- "$OUTPUT_ROOT" && pwd -P)

if [[ -z "$OUTPUT_ROOT" || "$OUTPUT_ROOT" == "/" || "$OUTPUT_ROOT" == "." || "$OUTPUT_ROOT" == ".." ]]; then
    argument_error "unsafe output directory: $OUTPUT_ROOT"
fi

TARGET_OUTPUT_DIR="$OUTPUT_ROOT/$TARGET_DIR_NAME"
PACKAGE_ID=$(cd "$SCRIPT_DIR" && cargo pkgid)
VERSION="${PACKAGE_ID##*@}"

if [[ -z "$VERSION" || "$VERSION" == "$PACKAGE_ID" ]]; then
    printf 'error: could not derive package version from: %s\n' "$PACKAGE_ID" >&2
    exit 1
fi

ARCHIVE_NAME="rtsql-v$VERSION-$TARGET_TRIPLE.tar.gz"
TARGET_BINARY="$SCRIPT_DIR/target/$TARGET_TRIPLE/release/rtsql"

if ! (cd "$SCRIPT_DIR" && CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER="$CROSS_LINKER" cargo rustc --locked --release --bin rtsql --target "$TARGET_TRIPLE" -- -C strip=symbols -C target-feature=+crt-static); then
    printf 'error: cross-build failed for %s\n' "$TARGET_TRIPLE" >&2
    exit 1
fi

if [[ ! -f "$TARGET_BINARY" ]]; then
    printf 'error: cross-build output not found: %s\n' "$TARGET_BINARY" >&2
    exit 1
fi

cleanup() {
    if [[ -n "$STAGING_DIR" && -d "$STAGING_DIR" ]]; then
        rm -rf -- "$STAGING_DIR"
    fi
    if [[ -n "$BACKUP_DIR" && -e "$BACKUP_DIR" && ! -e "$TARGET_OUTPUT_DIR" ]]; then
        mv -- "$BACKUP_DIR" "$TARGET_OUTPUT_DIR"
    fi
}

trap cleanup EXIT INT TERM

STAGING_DIR=$(mktemp -d "$OUTPUT_ROOT/.$TARGET_DIR_NAME.tmp.XXXXXX")
cp -- "$TARGET_BINARY" "$STAGING_DIR/rtsql"
chmod 0755 "$STAGING_DIR/rtsql"
cp -- "$SCRIPT_DIR/README.md" "$STAGING_DIR/README.md"
cp -- "$SCRIPT_DIR/README.zh-CN.md" "$STAGING_DIR/README.zh-CN.md"

tar -C "$STAGING_DIR" -czf "$STAGING_DIR/$ARCHIVE_NAME" rtsql README.md README.zh-CN.md
(
    cd "$STAGING_DIR"
    sha256sum "$ARCHIVE_NAME" > SHA256SUMS
)

if [[ -e "$TARGET_OUTPUT_DIR" ]]; then
    BACKUP_DIR=$(mktemp -d "$OUTPUT_ROOT/.$TARGET_DIR_NAME.backup.XXXXXX")
    rmdir "$BACKUP_DIR"
    mv -- "$TARGET_OUTPUT_DIR" "$BACKUP_DIR"
fi

if ! mv -- "$STAGING_DIR" "$TARGET_OUTPUT_DIR"; then
    printf 'error: could not publish output directory: %s\n' "$TARGET_OUTPUT_DIR" >&2
    exit 1
fi
STAGING_DIR=""

if [[ -n "$BACKUP_DIR" ]]; then
    rm -rf -- "$BACKUP_DIR"
    BACKUP_DIR=""
fi

trap - EXIT INT TERM

printf 'binary: %s\n' "$TARGET_OUTPUT_DIR/rtsql"
printf 'archive: %s\n' "$TARGET_OUTPUT_DIR/$ARCHIVE_NAME"
printf 'checksum: %s\n' "$TARGET_OUTPUT_DIR/SHA256SUMS"
printf '%s\n' 'cross-build succeeded; target execution not verified'
