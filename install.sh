#!/usr/bin/env bash
set -euo pipefail
umask 077

readonly REPOSITORY="VitorHolandaI/perfo"
readonly INSTALL_DIR="${PERFO_INSTALL_DIR:-$HOME/.local/bin}"
readonly BINARY_URL="https://raw.githubusercontent.com/$REPOSITORY/main/bin/perfo"

fail() {
    printf 'perfo installer error: %s\n' "$1" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

case "$(uname -s)" in
    Linux) ;;
    *) fail "unsupported operating system: $(uname -s) (expected Linux)" ;;
esac

case "$(uname -m)" in
    x86_64|amd64) ;;
    *) fail "unsupported architecture: $(uname -m) (expected x86_64)" ;;
esac

temporary_dir="$(mktemp -d)"
trap 'rm -rf "$temporary_dir"' EXIT

# If cargo is available and we are inside the source repo, allow building from source if requested
if [[ "${1:-}" == "--build" ]] && command -v cargo >/dev/null 2>&1 && [[ -f "Cargo.toml" ]]; then
    printf 'Building perfo from source with cargo...\n'
    cargo build --release --locked
    cp target/release/perfo "$temporary_dir/perfo"
else
    require_command curl
    printf 'Downloading pre-compiled perfo binary from %s...\n' "$REPOSITORY"
    curl --fail --location --silent --show-error \
        --proto '=https' --tlsv1.2 --retry 3 --connect-timeout 10 --max-time 120 \
        "$BINARY_URL" \
        --output "$temporary_dir/perfo"
fi

# Verify it is a valid ELF executable
if ! head -c 4 "$temporary_dir/perfo" | grep -q $'\x7fELF'; then
    fail "downloaded file is not a valid Linux ELF executable"
fi

mkdir -p "$INSTALL_DIR"
install --mode 0755 "$temporary_dir/perfo" "$INSTALL_DIR/perfo"

printf '\n✓ Successfully installed perfo to %s/perfo\n' "$INSTALL_DIR"

# Print version
if "$INSTALL_DIR/perfo" --version >/dev/null 2>&1; then
    printf '  Version: %s\n' "$("$INSTALL_DIR/perfo" --version)"
fi

# PATH check
case ":${PATH}:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        printf '\nNotice: %s is not in your PATH.\n' "$INSTALL_DIR"
        printf 'Add this to your shell profile (~/.bashrc, ~/.zshrc):\n'
        printf '  export PATH="%s:$PATH"\n' "$INSTALL_DIR"
        ;;
esac

printf '\nRun "perfo" to start the interactive monitor in your terminal.\n'
