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

URL="https://github.com/$REPO/releases/latest/download/devup-$ARCH-linux"
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

echo "Baixando $URL"
curl -fsSL "$URL" -o "$TMP"
mkdir -p "$BIN_DIR"
install -m 755 "$TMP" "$BIN_DIR/devup"
echo "devup instalado em $BIN_DIR/devup"

case ":$PATH:" in
  *":$BIN_DIR:"*) echo "Rode: devup" ;;
  *) echo "Adicione ao PATH: export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac
