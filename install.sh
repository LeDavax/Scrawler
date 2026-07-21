#!/bin/sh
set -e

REPO="LeDavax/Scrawler"
INSTALL_DIR="/usr/local/bin"

# Detect OS
OS="$(uname -s)"
case "$OS" in
  Darwin) os="darwin" ;;
  Linux)  os="linux" ;;
  *)      echo "Unsupported OS: $OS"; exit 1 ;;
esac

# Detect architecture
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)  arch="x86_64" ;;
  arm64|aarch64) arch="aarch64" ;;
  *)             echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

TARBALL="scrawler-${os}-${arch}.tar.gz"
URL="https://github.com/${REPO}/releases/latest/download/${TARBALL}"

echo "Downloading scrawler for ${os}/${arch}..."
TMP="$(mktemp -d)"
curl -fsSL "$URL" -o "$TMP/$TARBALL"
tar -xzf "$TMP/$TARBALL" -C "$TMP"

echo "Installing to $INSTALL_DIR..."
if [ -w "$INSTALL_DIR" ]; then
  mv "$TMP/scrawler" "$INSTALL_DIR/scrawler"
else
  sudo mv "$TMP/scrawler" "$INSTALL_DIR/scrawler"
fi
chmod +x "$INSTALL_DIR/scrawler"

rm -rf "$TMP"

echo "scrawler installed successfully!"
echo "Run 'scrawler --help' to get started."
