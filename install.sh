#!/bin/sh
set -eu

LWID_VERSION="${LWID_VERSION:-latest}"
INSTALL_DIR="${HOME}/.local/bin"
BINARY_NAME="lwid"
REPO="Marlinski/lwid"

# Detect OS
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
case "${OS}" in
  linux|darwin) ;;
  *) printf "Error: unsupported OS: %s\n" "${OS}" >&2; exit 1 ;;
esac

# Detect architecture
ARCH="$(uname -m)"
case "${ARCH}" in
  x86_64)  ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) printf "Error: unsupported architecture: %s\n" "${ARCH}" >&2; exit 1 ;;
esac

# Build download URL
ASSET="${BINARY_NAME}-${OS}-${ARCH}"
if [ "${LWID_VERSION}" = "latest" ]; then
  URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"
else
  URL="https://github.com/${REPO}/releases/download/${LWID_VERSION}/${ASSET}"
fi

# Install
mkdir -p "${INSTALL_DIR}"
printf "Downloading %s...\n" "${ASSET}"
curl -fSL --progress-bar -o "${INSTALL_DIR}/${BINARY_NAME}" "${URL}"
chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

printf "\nlwid installed to %s/%s\n" "${INSTALL_DIR}" "${BINARY_NAME}"

# PATH hint
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *) printf "\nAdd %s to your PATH:\n  export PATH=\"%s:\$PATH\"\n" "${INSTALL_DIR}" "${INSTALL_DIR}" ;;
esac
