#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PREFIX="${HOME}/.local"
NO_COMPLETIONS=0
UNINSTALL=0
PURGE_DATA=0

usage() {
    printf '%s\n' \
        'Usage:' \
        '  ./install.sh [--prefix DIR] [--no-completions]' \
        '  ./install.sh --uninstall [--purge-data] [--prefix DIR]' \
        '  ./install.sh --help'
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --help|-h)
            usage
            exit 0
            ;;
        --prefix)
            if [[ $# -lt 2 ]]; then
                printf '%s\n' 'error: --prefix requires a directory' >&2
                usage >&2
                exit 2
            fi
            PREFIX="$2"
            shift 2
            ;;
        --no-completions)
            NO_COMPLETIONS=1
            shift
            ;;
        --uninstall)
            UNINSTALL=1
            shift
            ;;
        --purge-data)
            PURGE_DATA=1
            shift
            ;;
        *)
            printf 'error: unknown option: %s\n' "$1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [[ "$PURGE_DATA" -eq 1 && "$UNINSTALL" -eq 0 ]]; then
    printf '%s\n' 'error: --purge-data requires --uninstall' >&2
    usage >&2
    exit 2
fi

DATA_DIR="${RTSQL_HOME:-$HOME/.rtsql}"
BIN_PATH="$PREFIX/bin/rtsql"
BASH_COMPLETION="$HOME/.local/share/bash-completion/completions/rtsql"
ZSH_COMPLETION="$HOME/.zsh/completions/_rtsql"
FISH_COMPLETION="$HOME/.config/fish/completions/rtsql.fish"
BASHRC_PATH="$HOME/.bashrc"

if [[ "$PREFIX" == "$HOME/.local" ]]; then
    PATH_EXPORT_LINE='export PATH="$HOME/.local/bin:$PATH"'
else
    printf -v PATH_DIR_QUOTED '%q' "$PREFIX/bin"
    PATH_EXPORT_LINE="export PATH=${PATH_DIR_QUOTED}:\$PATH"
fi
BASHRC_PATH_UPDATED=0

remove_program() {
    rm -f -- "$BIN_PATH" "$BASH_COMPLETION" "$ZSH_COMPLETION" "$FISH_COMPLETION"
}

bashrc_contains_path_line() {
    local line

    [[ -f "$BASHRC_PATH" ]] || return 1
    while IFS= read -r line || [[ -n "$line" ]]; do
        [[ "$line" == "$PATH_EXPORT_LINE" ]] && return 0
    done < "$BASHRC_PATH"
    return 1
}

ensure_bashrc_path() {
    BASHRC_PATH_UPDATED=0
    if [[ ! -f "$BASHRC_PATH" ]]; then
        printf '%s\n' "$PATH_EXPORT_LINE" > "$BASHRC_PATH"
        BASHRC_PATH_UPDATED=1
    elif ! bashrc_contains_path_line; then
        printf '\n%s\n' "$PATH_EXPORT_LINE" >> "$BASHRC_PATH"
        BASHRC_PATH_UPDATED=1
    fi
}

remove_bashrc_path() {
    local line
    local filtered
    local found=0

    bashrc_contains_path_line || return 0
    filtered=$(mktemp "${BASHRC_PATH}.rtsql.XXXXXX")
    while IFS= read -r line || [[ -n "$line" ]]; do
        if [[ "$line" == "$PATH_EXPORT_LINE" ]]; then
            found=1
        else
            printf '%s\n' "$line" >> "$filtered"
        fi
    done < "$BASHRC_PATH"

    if [[ "$found" -eq 0 ]]; then
        rm -f -- "$filtered"
        return 0
    fi

    cat "$filtered" > "$BASHRC_PATH"
    rm -f -- "$filtered"
}

if [[ "$UNINSTALL" -eq 1 ]]; then
    remove_program
    remove_bashrc_path
    if [[ "$PURGE_DATA" -eq 1 ]]; then
        case "$DATA_DIR" in
            ''|/|.|..)
                printf 'error: refusing to purge invalid data directory: %s\n' "$DATA_DIR" >&2
                exit 1
                ;;
        esac
        printf 'will remove data directory: %s\n' "$DATA_DIR"
        rm -rf -- "$DATA_DIR"
    fi
    exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
    printf '%s\n' 'error: cargo is required' >&2
    exit 1
fi

if ! (cd "$SCRIPT_DIR" && cargo build --release); then
    printf '%s\n' 'error: cargo build --release failed' >&2
    exit 1
fi

SOURCE_BIN="$SCRIPT_DIR/target/release/rtsql"
if [[ ! -x "$SOURCE_BIN" ]]; then
    printf 'error: release binary not found: %s\n' "$SOURCE_BIN" >&2
    exit 1
fi

if command -v strip >/dev/null 2>&1; then
    if ! strip "$SOURCE_BIN"; then
        printf '%s\n' 'warning: strip failed; continuing without stripping' >&2
    fi
else
    printf '%s\n' 'warning: strip not found; continuing without stripping' >&2
fi

mkdir -p "$PREFIX/bin"
install -m 0755 "$SOURCE_BIN" "$BIN_PATH"

if [[ "$NO_COMPLETIONS" -eq 0 ]]; then
    shell_name="${SHELL:-}"
    shell_name="${shell_name##*/}"
    case "$shell_name" in
        bash)
            if command -v bash >/dev/null 2>&1; then
                mkdir -p "$(dirname "$BASH_COMPLETION")"
                "$BIN_PATH" completions bash > "$BASH_COMPLETION"
            fi
            ;;
        zsh)
            if command -v zsh >/dev/null 2>&1; then
                mkdir -p "$(dirname "$ZSH_COMPLETION")"
                "$BIN_PATH" completions zsh > "$ZSH_COMPLETION"
                printf 'add this directory to your fpath: %s\n' "$(dirname "$ZSH_COMPLETION")"
            fi
            ;;
        fish)
            if command -v fish >/dev/null 2>&1; then
                mkdir -p "$(dirname "$FISH_COMPLETION")"
                "$BIN_PATH" completions fish > "$FISH_COMPLETION"
            fi
            ;;
    esac
fi

ensure_bashrc_path

if [[ "$BASHRC_PATH_UPDATED" -eq 1 ]]; then
    printf 'added this directory to your PATH in %s: %s\n' "$BASHRC_PATH" "$PREFIX/bin"
else
    case ":$PATH:" in
        *":$PREFIX/bin:"*) ;;
        *) printf 'add this directory to your PATH: %s\n' "$PREFIX/bin" ;;
    esac
fi
