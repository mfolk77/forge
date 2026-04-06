#!/bin/sh
set -e

REPO="mfolk77/forge"
INSTALL_DIR="${FORGE_INSTALL_DIR:-$HOME/.local/bin}"

# Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin)
    case "$ARCH" in
      arm64) ASSET="forge-macos-arm64.tar.gz" ;;
      x86_64) ASSET="forge-macos-x86_64.tar.gz" ;;
      *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    ;;
  Linux)
    case "$ARCH" in
      x86_64) ASSET="forge-linux-x86_64.tar.gz" ;;
      *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    ;;
  *)
    echo "Unsupported OS: $OS (use install.ps1 for Windows)"
    exit 1
    ;;
esac

# Get latest release tag
if command -v curl >/dev/null 2>&1; then
  LATEST=$(curl -sI "https://github.com/$REPO/releases/latest" | grep -i ^location: | sed 's/.*tag\///' | tr -d '\r\n')
elif command -v wget >/dev/null 2>&1; then
  LATEST=$(wget --spider --max-redirect=0 "https://github.com/$REPO/releases/latest" 2>&1 | grep Location | sed 's/.*tag\///' | tr -d '\r\n')
else
  echo "Error: curl or wget required"
  exit 1
fi

if [ -z "$LATEST" ]; then
  echo "Error: could not determine latest release"
  exit 1
fi

URL="https://github.com/$REPO/releases/download/$LATEST/$ASSET"

echo "Installing Forge $LATEST for $OS/$ARCH..."
echo "  From: $URL"
echo "  To:   $INSTALL_DIR/forge"

# Create install directory
mkdir -p "$INSTALL_DIR"

# Download and extract
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

if command -v curl >/dev/null 2>&1; then
  curl -fSL "$URL" -o "$TMPDIR/$ASSET"
else
  wget -q "$URL" -O "$TMPDIR/$ASSET"
fi

tar xzf "$TMPDIR/$ASSET" -C "$TMPDIR"

# Binary inside the archive is named 'forge'
if [ -f "$TMPDIR/forge" ]; then
  mv "$TMPDIR/forge" "$INSTALL_DIR/forge"
else
  # Fallback: find any forge binary in the tmp dir
  find "$TMPDIR" -name "forge" -type f -exec mv {} "$INSTALL_DIR/forge" \; 2>/dev/null
fi

chmod +x "$INSTALL_DIR/forge"

echo ""
echo "Forge installed to $INSTALL_DIR/forge"

# Check if install dir is in PATH
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo ""
    echo "Add this to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac

echo ""
echo "Run 'forge setup' to install the backend and download a model."
echo "  (This is a one-time setup — it detects your hardware automatically.)"
