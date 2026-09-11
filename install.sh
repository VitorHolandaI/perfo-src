#!/bin/sh
set -eu

umask 022

REPOSITORY="${PERFO_REPOSITORY:-VitorHolandaI/perfo}"

# A released tag, never a branch. A branch moves, so "the binary I audited"
# and "the binary this installs" would not be the same thing tomorrow. Set
# PERFO_VERSION to install a different release.
VERSION="${PERFO_VERSION:-0.1.3}"

if [ -n "${PERFO_INSTALL_DIR:-}" ]; then
    INSTALL_DIR="$PERFO_INSTALL_DIR"
elif [ "$(id -u 2>/dev/null || echo 1)" = "0" ]; then
    INSTALL_DIR="/usr/local/bin"
else
    INSTALL_DIR="${HOME}/.local/bin"
fi

fail() {
    printf 'perfo installer error: %s\n' "$1" >&2
    exit 1
}

if [ "${1:-}" = "--uninstall" ]; then
    if [ -f "$INSTALL_DIR/perfo" ]; then
        rm -f "$INSTALL_DIR/perfo"
        printf 'perfo removed from %s/perfo\n' "$INSTALL_DIR"
    else
        printf 'perfo not found in %s\n' "$INSTALL_DIR"
    fi
    exit 0
fi

case "$(uname -s)" in
    Linux) ;;
    *) fail "unsupported operating system: $(uname -s) (Linux required)" ;;
esac

case "$(uname -m)" in
    x86_64|amd64) ;;
    *) fail "unsupported architecture: $(uname -m) (x86_64 required)" ;;
esac

tmp_dir="$(mktemp -d 2>/dev/null || mktemp -d -t 'perfo-install.XXXXXX')"
cleanup() {
    rm -rf "$tmp_dir"
}
trap cleanup EXIT INT TERM

tmp_binary="$tmp_dir/perfo"

if [ "${1:-}" = "--build" ] && command -v cargo >/dev/null 2>&1 && [ -f "Cargo.toml" ]; then
    printf 'Building perfo from source with cargo...\n'
    cargo build --release --locked
    cp target/release/perfo "$tmp_binary"
else
    BASE="https://github.com/${REPOSITORY}/releases/download/v${VERSION}"
    tarball="$tmp_dir/perfo-linux-x86_64.tar.gz"
    sums="$tmp_dir/perfo-linux-x86_64.sha256"
    printf 'Downloading perfo v%s from %s...\n' "$VERSION" "$REPOSITORY"

    fetch() {
        if command -v curl >/dev/null 2>&1; then
            curl --fail --location --silent --show-error \
                --proto '=https' --tlsv1.2 --retry 3 --connect-timeout 10 --max-time 120 \
                "$1" --output "$2" || fail "failed to download $1"
        elif command -v wget >/dev/null 2>&1; then
            wget --quiet --timeout=120 --tries=3 -O "$2" "$1" \
                || fail "failed to download $1"
        else
            fail "neither curl nor wget found; please install curl or wget"
        fi
    }

    fetch "$BASE/perfo-linux-x86_64.tar.gz" "$tarball"
    fetch "$BASE/perfo-linux-x86_64.sha256" "$sums"

    # The checksum is published alongside the archive by the release workflow,
    # which also attests the binary's provenance. Verifying it here catches a
    # truncated download and a tampered mirror; it is not a substitute for
    # trusting the release itself.
    if command -v sha256sum >/dev/null 2>&1; then
        expected="$(cut -d' ' -f1 < "$sums")"
        actual="$(sha256sum "$tarball" | cut -d' ' -f1)"
    elif command -v shasum >/dev/null 2>&1; then
        expected="$(cut -d' ' -f1 < "$sums")"
        actual="$(shasum -a 256 "$tarball" | cut -d' ' -f1)"
    else
        fail "neither sha256sum nor shasum found; cannot verify the download"
    fi
    [ -n "$expected" ] || fail "published checksum is empty"
    if [ "$expected" != "$actual" ]; then
        fail "checksum mismatch: expected $expected, got $actual"
    fi
    printf 'Checksum verified: %s\n' "$actual"

    tar --extract --gzip --file "$tarball" --directory "$tmp_dir" perfo \
        || fail "failed to extract the archive"
fi

# Verify it is a valid ELF executable (magic number 7f 45 4c 46)
if command -v od >/dev/null 2>&1; then
    magic="$(od -An -tx1 -N4 "$tmp_binary" 2>/dev/null | tr -d ' \n')"
    if [ "$magic" != "7f454c46" ]; then
        fail "downloaded file is not a valid Linux ELF executable"
    fi
elif command -v file >/dev/null 2>&1; then
    if ! file "$tmp_binary" | grep -qi "ELF.*executable"; then
        fail "downloaded file is not a valid Linux ELF executable"
    fi
fi

chmod +x "$tmp_binary"

mkdir -p "$INSTALL_DIR"
if command -v install >/dev/null 2>&1; then
    install -m 0755 "$tmp_binary" "$INSTALL_DIR/perfo"
else
    cp "$tmp_binary" "$INSTALL_DIR/perfo"
    chmod 0755 "$INSTALL_DIR/perfo"
fi

printf '\n✓ Successfully installed perfo to %s/perfo\n' "$INSTALL_DIR"

if "$INSTALL_DIR/perfo" --version >/dev/null 2>&1; then
    printf '  Version: %s\n' "$("$INSTALL_DIR/perfo" --version)"
fi

case ":${PATH}:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        printf '\nNotice: %s is not in your PATH.\n' "$INSTALL_DIR"
        printf 'Add this to your shell profile (~/.bashrc, ~/.zshrc):\n'
        printf '  export PATH="%s:$PATH"\n' "$INSTALL_DIR"
        ;;
esac

printf '\nRun "perfo" to launch the system monitor TUI.\n'
