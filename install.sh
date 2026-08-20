#!/bin/sh
# synapse install script — download and install the correct binary for your OS/arch.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Kodjaoglanian/synapse/main/install.sh | sh
#
# Or to install a specific version:
#   curl -fsSL https://raw.githubusercontent.com/Kodjaoglanian/synapse/main/install.sh | sh -s -- --version v0.1.0
#
# Flags:
#   --version <tag>   Install a specific release tag (default: latest)
#   --prefix <dir>    Install prefix (default: /usr/local)
#   --no-sudo         Don't use sudo even if available

set -eu

REPO="Kodjaoglanian/synapse"
VERSION="latest"
PREFIX="/usr/local"
USE_SUDO=1

# --- Parse args ---
while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="$2"; shift 2 ;;
        --prefix)  PREFIX="$2";  shift 2 ;;
        --no-sudo) USE_SUDO=0;   shift   ;;
        --help|-h)
            echo "Usage: $0 [--version <tag>] [--prefix <dir>] [--no-sudo]"
            exit 0
            ;;
        *) echo "Unknown flag: $1" >&2; exit 1 ;;
    esac
done

# --- Detect OS ---
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)  OS_TARGET="unknown-linux-gnu" ;;
    Darwin) OS_TARGET="apple-darwin" ;;
    MINGW*|MSYS*|CYGWIN*)
        echo "On Windows, please download the .zip from the releases page:" >&2
        echo "  https://github.com/$REPO/releases" >&2
        exit 1
        ;;
    *) echo "Unsupported OS: $OS" >&2; exit 1 ;;
esac

case "$ARCH" in
    x86_64|amd64)  ARCH_TARGET="x86_64" ;;
    arm64|aarch64) ARCH_TARGET="aarch64" ;;
    *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

TARGET="${ARCH_TARGET}-${OS_TARGET}"

# --- Resolve version ---
if [ "$VERSION" = "latest" ]; then
    echo ">>> Fetching latest release tag..."
    VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
    if [ -z "$VERSION" ]; then
        echo "Could not determine the latest release. Specify --version manually." >&2
        exit 1
    fi
fi

echo ">>> Installing synapse $VERSION for $TARGET"

# --- Determine download URL ---
ASSET_NAME="synapse-${VERSION}-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/$REPO/releases/download/${VERSION}/${ASSET_NAME}"

echo ">>> Downloading $DOWNLOAD_URL"

# --- Download to a temp directory ---
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

if ! curl -fsSL "$DOWNLOAD_URL" -o "$TMP_DIR/$ASSET_NAME"; then
    echo "Download failed. The asset may not exist for $TARGET." >&2
    echo "Check available assets at: https://github.com/$REPO/releases/tag/$VERSION" >&2
    exit 1
fi

echo ">>> Extracting..."
tar xzf "$TMP_DIR/$ASSET_NAME" -C "$TMP_DIR"

# The archive contains a directory like synapse-v0.1.0-x86_64-unknown-linux-gnu/
BINARY_PATH=$(find "$TMP_DIR" -name synapse -type f | head -1)
if [ -z "$BINARY_PATH" ]; then
    echo "Binary not found in archive." >&2
    exit 1
fi

# --- Install ---
SUDO=""
if [ "$USE_SUDO" = "1" ] && [ "$(id -u)" != "0" ] && command -v sudo >/dev/null 2>&1; then
    SUDO="sudo"
fi

BIN_DIR="${PREFIX}/bin"
if [ ! -d "$BIN_DIR" ]; then
    $SUDO mkdir -p "$BIN_DIR"
fi

echo ">>> Installing to ${BIN_DIR}/synapse"
$SUDO install -m 0755 "$BINARY_PATH" "${BIN_DIR}/synapse"

# --- Verify ---
if command -v synapse >/dev/null 2>&1; then
    echo ""
    echo "✓ synapse installed successfully!"
    echo ""
    synapse --version 2>/dev/null || synapse --help 2>&1 | head -1
    echo ""
    echo "Run 'synapse --help' to get started."
else
    echo ""
    echo "✓ synapse installed to ${BIN_DIR}/synapse"
    echo "  Add ${BIN_DIR} to your PATH to use it."
fi
