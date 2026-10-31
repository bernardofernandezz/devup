#!/bin/sh
# Instala o devup: curl -fsSL https://raw.githubusercontent.com/bernardofernandezz/devup/main/install.sh | sh
set -eu

REPO="bernardofernandezz/devup"
BIN_DIR="${DEVUP_BIN_DIR:-$HOME/.local/bin}"

case "$(uname -m)" in
  x86_64 | amd64) ARCH=x86_64 ;;
  aarch64 | arm64) ARCH=aarch64 ;;
  *) echo "Arquitetura não suportada: $(uname -m)" >&2; exit 1 ;;
esac

ASSET="devup-$ARCH-linux"
BASE="https://github.com/$REPO/releases/latest/download"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Baixando $BASE/$ASSET"
curl -fsSL "$BASE/$ASSET" -o "$TMP/$ASSET"

if command -v sha256sum >/dev/null; then
  curl -fsSL "$BASE/sha256sums.txt" -o "$TMP/sha256sums.txt"
  (cd "$TMP" && grep " $ASSET\$" sha256sums.txt | sha256sum -c - >/dev/null) || {
    echo "Checksum não confere; abortando." >&2
    exit 1
  }
  echo "Checksum ok"
fi

mkdir -p "$BIN_DIR"
install -m 755 "$TMP/$ASSET" "$BIN_DIR/devup"
echo "devup instalado em $BIN_DIR/devup"

case ":$PATH:" in
  *":$BIN_DIR:"*) echo "Comece com: devup init" ;;
  *) echo "Adicione ao PATH: export PATH=\"$BIN_DIR:\$PATH\"  e depois rode: devup init" ;;
esac
