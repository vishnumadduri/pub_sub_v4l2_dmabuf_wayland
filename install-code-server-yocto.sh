#!/bin/sh
set -eu

usage() {
  cat <<USAGE
Yocto/BusyBox installer for code-server.

Usage:
  $0 [--dry-run] [--version X.Y.Z] [--edge] [--method detect|opkg|standalone] [--prefix DIR]

Options:
  --dry-run                Print commands without executing.
  --version X.Y.Z          Install a specific code-server version.
  --edge                   Install latest edge release tag.
  --method detect          Try opkg first, then standalone (default).
  --method opkg            Use opkg only.
  --method standalone      Use standalone tarball only.
  --prefix DIR             Install prefix for standalone mode (default: /usr/local).
  -h, --help               Show this help.
USAGE
}

echoh() {
  echo "$@"
}

echoerr() {
  echo "$@" >&2
}

command_exists() {
  command -v "$1" >/dev/null 2>&1
}

sh_c() {
  echoh "+ $*"
  if [ -z "${DRY_RUN:-}" ]; then
    sh -c "$*"
  fi
}

sudo_sh_c() {
  if [ "$(id -u)" = "0" ]; then
    sh_c "$@"
  elif command_exists sudo; then
    sh_c "sudo $*"
  elif command_exists doas; then
    sh_c "doas $*"
  elif command_exists su; then
    sh_c "su root -c '$*'"
  else
    echoerr "Need root privileges for: $*"
    echoerr "Please run as root or install sudo/doas/su."
    exit 1
  fi
}

http_get() {
  URL="$1"
  if command_exists curl; then
    curl -fsSL "$URL"
  elif command_exists wget; then
    wget -qO- "$URL"
  else
    echoerr "curl or wget is required."
    exit 1
  fi
}

fetch_file() {
  URL="$1"
  OUT="$2"
  if [ -f "$OUT" ]; then
    echoh "+ Reusing $OUT"
    return
  fi
  mkdir -p "$(dirname "$OUT")"
  if command_exists curl; then
    sh_c "curl -fL -o '$OUT.incomplete' '$URL'"
  elif command_exists wget; then
    sh_c "wget -O '$OUT.incomplete' '$URL'"
  else
    echoerr "curl or wget is required."
    exit 1
  fi
  sh_c "mv '$OUT.incomplete' '$OUT'"
}

arch() {
  U="$(uname -m)"
  case "$U" in
    aarch64|arm64) echo arm64 ;;
    x86_64|amd64) echo amd64 ;;
    armv7l|armv7) echo armv7l ;;
    *) echo "$U" ;;
  esac
}

echo_latest_version() {
  if [ -n "${EDGE:-}" ]; then
    TAG="$(http_get "https://api.github.com/repos/coder/code-server/releases" | sed -n 's/.*"tag_name": "v\{0,1\}\([^"]*\)".*/\1/p' | head -n 1)"
  else
    TAG="$(http_get "https://api.github.com/repos/coder/code-server/releases/latest" | sed -n 's/.*"tag_name": "v\{0,1\}\([^"]*\)".*/\1/p' | head -n 1)"
  fi
  if [ -z "$TAG" ]; then
    echoerr "Unable to determine latest version from GitHub API."
    exit 1
  fi
  echo "$TAG"
}

install_opkg() {
  if ! command_exists opkg; then
    echoerr "opkg is not available on this system."
    return 1
  fi

  PKG="code-server"
  if [ -n "${VERSION:-}" ]; then
    PKG="code-server=${VERSION}"
  fi

  echoh "Installing with opkg (${PKG})."
  sudo_sh_c "opkg update"
  if sudo_sh_c "opkg install '$PKG'"; then
    echoh
    echoh "code-server installed via opkg."
    echoh "Run with: code-server"
    return 0
  fi

  echoerr "opkg install failed for ${PKG}."
  return 1
}

install_standalone() {
  INSTALL_PREFIX="${STANDALONE_INSTALL_PREFIX:-/usr/local}"
  CACHE_DIR="${XDG_CACHE_HOME:-${HOME:-/tmp}/.cache}/code-server"
  OS=linux
  ARCH="$(arch)"

  URL="https://github.com/coder/code-server/releases/download/v${VERSION}/code-server-${VERSION}-${OS}-${ARCH}.tar.gz"
  FILE="$CACHE_DIR/code-server-${VERSION}-${OS}-${ARCH}.tar.gz"

  echoh "Installing v${VERSION} standalone for ${OS}/${ARCH}."
  fetch_file "$URL" "$FILE"

  sh_c "mkdir -p '$INSTALL_PREFIX'"

  RUN_AS="sh_c"
  if [ ! -w "$INSTALL_PREFIX" ] && [ -z "${DRY_RUN:-}" ]; then
    RUN_AS="sudo_sh_c"
  fi

  "$RUN_AS" "mkdir -p '$INSTALL_PREFIX/lib' '$INSTALL_PREFIX/bin'"
  "$RUN_AS" "tar -C '$INSTALL_PREFIX/lib' -xzf '$FILE'"
  "$RUN_AS" "rm -rf '$INSTALL_PREFIX/lib/code-server-${VERSION}'"
  "$RUN_AS" "mv '$INSTALL_PREFIX/lib/code-server-${VERSION}-${OS}-${ARCH}' '$INSTALL_PREFIX/lib/code-server-${VERSION}'"
  "$RUN_AS" "ln -fs '$INSTALL_PREFIX/lib/code-server-${VERSION}/bin/code-server' '$INSTALL_PREFIX/bin/code-server'"

  echoh
  echoh "Standalone install complete."
  echoh "Binary: $INSTALL_PREFIX/bin/code-server"
}

main() {
  METHOD="detect"
  STANDALONE_INSTALL_PREFIX="/usr/local"

  while [ "$#" -gt 0 ]; do
    case "$1" in
      --dry-run)
        DRY_RUN=1
        ;;
      --version)
        shift
        [ "$#" -gt 0 ] || { echoerr "--version requires an argument"; exit 1; }
        VERSION="$1"
        ;;
      --version=*)
        VERSION="${1#*=}"
        ;;
      --edge)
        EDGE=1
        ;;
      --method)
        shift
        [ "$#" -gt 0 ] || { echoerr "--method requires an argument"; exit 1; }
        METHOD="$1"
        ;;
      --method=*)
        METHOD="${1#*=}"
        ;;
      --prefix)
        shift
        [ "$#" -gt 0 ] || { echoerr "--prefix requires an argument"; exit 1; }
        STANDALONE_INSTALL_PREFIX="$1"
        ;;
      --prefix=*)
        STANDALONE_INSTALL_PREFIX="${1#*=}"
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        echoerr "Unknown argument: $1"
        usage
        exit 1
        ;;
    esac
    shift
  done

  case "$METHOD" in
    detect|opkg|standalone) ;;
    *)
      echoerr "Invalid --method: $METHOD"
      usage
      exit 1
      ;;
  esac

  VERSION="${VERSION:-$(echo_latest_version)}"

  case "$METHOD" in
    opkg)
      install_opkg
      ;;
    standalone)
      install_standalone
      ;;
    detect)
      if install_opkg; then
        :
      else
        echoh "Falling back to standalone install."
        install_standalone
      fi
      ;;
  esac

  echoh "Done."
}

main "$@"
