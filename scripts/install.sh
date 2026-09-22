#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

# Usage:
#   curl -fsSL https://raw.githubusercontent.com/seuros/skipper/master/scripts/install.sh | bash
#
# Asset names are a contract with .github/workflows/release-binaries.yml:
# skipper-mcp-${tag}-${triple}.tar.gz, binary at the archive root.

REPO="seuros/skipper"
BIN="skipper-mcp"
DEFAULT_PREFIX="$HOME/.local/bin"

usage() {
  cat <<'EOF'
Usage: install.sh [options]

Options:
  --prefix <dir>    Install into this directory (default: ~/.local/bin)
  --version <tag>   Install a specific tag, with or without the leading v
  -h, --help        Show this help

Environment:
  SKIPPER_INSTALL_PREFIX    same as --prefix
  SKIPPER_INSTALL_VERSION   same as --version
EOF
}

info() { printf '==> %s\n' "$*"; }
warn() { printf 'WARN: %s\n' "$*" >&2; }
fail() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

parse_args() {
  VERSION="${SKIPPER_INSTALL_VERSION:-}"
  PREFIX="${SKIPPER_INSTALL_PREFIX:-}"

  while [ "$#" -gt 0 ]; do
    case "$1" in
      --prefix)
        [ "$#" -ge 2 ] || fail "--prefix requires a directory"
        PREFIX="$2"
        shift
        ;;
      --version)
        [ "$#" -ge 2 ] || fail "--version requires a tag"
        VERSION="$2"
        shift
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        fail "unknown option: $1 (use --help)"
        ;;
    esac
    shift
  done

  PREFIX="${PREFIX:-$DEFAULT_PREFIX}"
  case "$PREFIX" in
    "~") PREFIX="$HOME" ;;
    "~/"*) PREFIX="$HOME/${PREFIX#\~/}" ;;
  esac
}

detect_http() {
  if command -v curl >/dev/null 2>&1; then
    HTTP=curl
  elif command -v wget >/dev/null 2>&1; then
    HTTP=wget
  else
    fail "curl or wget is required"
  fi
}

fetch() {
  if [ "$HTTP" = curl ]; then
    curl -fsSL --proto '=https' --tlsv1.2 "$1"
  else
    wget -qO- "$1"
  fi
}

download() {
  if [ "$HTTP" = curl ]; then
    curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1"
  else
    wget -q -O "$2" "$1"
  fi
}

detect_target() {
  local os arch
  case "$(uname -s | tr '[:upper:]' '[:lower:]')" in
    linux) os=linux ;;
    darwin) os=darwin ;;
    *) fail "unsupported OS: $(uname -s)" ;;
  esac

  case "$(uname -m)" in
    x86_64|amd64) arch=x86_64 ;;
    aarch64|arm64) arch=aarch64 ;;
    *) fail "unsupported architecture: $(uname -m)" ;;
  esac

  case "${os}-${arch}" in
    linux-x86_64) TARGET=x86_64-unknown-linux-gnu ;;
    linux-aarch64) TARGET=aarch64-unknown-linux-gnu ;;
    darwin-x86_64) TARGET=x86_64-apple-darwin ;;
    darwin-aarch64) TARGET=aarch64-apple-darwin ;;
    *) fail "no binary for ${os}/${arch}" ;;
  esac

  PLATFORM="${os}/${arch}"
}

resolve_tag() {
  if [ -n "$VERSION" ]; then
    TAG="v${VERSION#v}"
    return
  fi

  local json
  json=$(fetch "https://api.github.com/repos/${REPO}/releases/latest") ||
    fail "could not reach the GitHub API to find the latest release"

  TAG=$(printf '%s\n' "$json" | awk -F'"' '/"tag_name":/ { print $4; exit }')
  [ -n "${TAG:-}" ] || fail "no published release found for ${REPO}"
}

install_binary() {
  command -v tar >/dev/null 2>&1 || fail "tar is required"

  local tmp asset url
  tmp=$(mktemp -d 2>/dev/null || mktemp -d -t skipper-install)
  trap 'rm -rf "$tmp"' EXIT

  asset="${BIN}-${TAG}-${TARGET}.tar.gz"
  url="https://github.com/${REPO}/releases/download/${TAG}/${asset}"

  info "skipper ${TAG} for ${PLATFORM}"
  download "$url" "${tmp}/${asset}" || fail "could not download ${url}"

  tar xzf "${tmp}/${asset}" -C "$tmp"
  [ -s "${tmp}/${BIN}" ] || fail "${BIN} missing from ${asset}"

  mkdir -p "$PREFIX" || fail "cannot create ${PREFIX}"
  [ -w "$PREFIX" ] || fail "${PREFIX} is not writable"
  install -m 0755 "${tmp}/${BIN}" "${PREFIX}/${BIN}"

  info "installed ${PREFIX}/${BIN}"
}

report_path() {
  case ":${PATH}:" in
    *":${PREFIX}:"*) return ;;
  esac

  warn "${PREFIX} is not on your PATH"
  printf '  export PATH="%s:$PATH"\n' "$PREFIX"
}

main() {
  command -v install >/dev/null 2>&1 || fail "'install' is required"
  parse_args "$@"
  detect_http
  detect_target
  resolve_tag
  install_binary
  report_path
}

main "$@"
