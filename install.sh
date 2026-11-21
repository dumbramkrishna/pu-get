#!/bin/sh
# pu-get installer for Linux and macOS.
#
# Downloads a published release archive, verifies it against the release's
# SHA256SUMS.txt, and installs the binary. HTTPS is required for GitHub
# downloads; plain HTTP is accepted only for a localhost mirror.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/dumbramkrishna/pu-get/main/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --version v0.0.1 --dir "$HOME/.local/bin"
#
# Environment:
#   PU_GET_VERSION        release tag to install (default: newest release)
#   PU_GET_INSTALL_DIR    install directory (default: ~/.local/bin)
#   PU_GET_REPO           override OWNER/REPO
#   PU_GET_API_BASE       override the GitHub API base URL
#   PU_GET_DOWNLOAD_BASE  override the release download base URL

set -eu

REPO="${PU_GET_REPO:-dumbramkrishna/pu-get}"
API_BASE="${PU_GET_API_BASE:-https://api.github.com}"
DOWNLOAD_BASE="${PU_GET_DOWNLOAD_BASE:-https://github.com}"
VERSION="${PU_GET_VERSION:-}"
INSTALL_DIR="${PU_GET_INSTALL_DIR:-}"
FORCE=0
DRY_RUN=0

usage() {
    cat <<'EOF'
pu-get installer

Usage: install.sh [options]

Options:
  -V, --version TAG   Release tag to install (default: newest release)
  -d, --dir DIR       Install directory (default: ~/.local/bin)
  -f, --force         Reinstall even if this version is already present
      --dry-run       Show what would be downloaded and where it would go
  -h, --help          Show this help

Environment:
  PU_GET_VERSION, PU_GET_INSTALL_DIR, PU_GET_REPO,
  PU_GET_API_BASE, PU_GET_DOWNLOAD_BASE

Examples:
  curl -fsSL https://raw.githubusercontent.com/dumbramkrishna/pu-get/main/install.sh | sh
  curl -fsSL .../install.sh | sh -s -- --version v0.0.1 --dir "$HOME/.local/bin"
EOF
}

note() {
    printf '%s\n' "$*" >&2
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

have() {
    command -v "$1" >/dev/null 2>&1
}

fetch_stdout() {
    url=$1
    if have curl; then
        case "$url" in
            https://*)
                curl -fsSL --retry 3 --retry-delay 2 --connect-timeout 15 \
                    --proto '=https' --tlsv1.2 "$url"
                ;;
            http://localhost:* | http://127.0.0.1:*)
                curl -fsSL --retry 3 --retry-delay 2 --connect-timeout 15 "$url"
                ;;
            *)
                die "refusing non-HTTPS URL: $url"
                ;;
        esac
    elif have wget; then
        case "$url" in
            https://*)
                wget -qO- --https-only --tries=3 --timeout=30 "$url"
                ;;
            http://localhost:* | http://127.0.0.1:*)
                wget -qO- --tries=3 --timeout=30 "$url"
                ;;
            *)
                die "refusing non-HTTPS URL: $url"
                ;;
        esac
    else
        die "curl or wget is required to download pu-get"
    fi
}

fetch_file() {
    url=$1
    dest=$2
    if have curl; then
        case "$url" in
            https://*)
                curl -fsSL --retry 3 --retry-delay 2 --connect-timeout 15 \
                    --proto '=https' --tlsv1.2 -o "$dest" "$url"
                ;;
            http://localhost:* | http://127.0.0.1:*)
                curl -fsSL --retry 3 --retry-delay 2 --connect-timeout 15 -o "$dest" "$url"
                ;;
            *)
                die "refusing non-HTTPS URL: $url"
                ;;
        esac
    elif have wget; then
        case "$url" in
            https://*)
                wget -q --https-only --tries=3 --timeout=30 -O "$dest" "$url"
                ;;
            http://localhost:* | http://127.0.0.1:*)
                wget -q --tries=3 --timeout=30 -O "$dest" "$url"
                ;;
            *)
                die "refusing non-HTTPS URL: $url"
                ;;
        esac
    else
        die "curl or wget is required to download pu-get"
    fi
}

while [ $# -gt 0 ]; do
    case "$1" in
        -h | --help)
            usage
            exit 0
            ;;
        -V | --version)
            [ $# -ge 2 ] || die "--version requires a tag, for example v0.0.1"
            VERSION=$2
            shift 2
            ;;
        --version=*)
            VERSION=${1#*=}
            shift
            ;;
        -d | --dir)
            [ $# -ge 2 ] || die "--dir requires a path"
            INSTALL_DIR=$2
            shift 2
            ;;
        --dir=*)
            INSTALL_DIR=${1#*=}
            shift
            ;;
        -f | --force)
            FORCE=1
            shift
            ;;
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --)
            shift
            break
            ;;
        *)
            die "unknown option: $1 (try --help)"
            ;;
    esac
done

[ $# -eq 0 ] || die "unexpected argument: $1"

case "${HOME:-}" in
    "") [ -n "$INSTALL_DIR" ] || die "HOME is not set; pass --dir DIR" ;;
    *) [ -n "$INSTALL_DIR" ] || INSTALL_DIR="$HOME/.local/bin" ;;
esac

case "$INSTALL_DIR" in
    "~/"*)
        [ -n "${HOME:-}" ] || die "HOME is not set; pass --dir DIR"
        INSTALL_DIR="${HOME}/${INSTALL_DIR#\~/}"
        ;;
esac
[ -n "$INSTALL_DIR" ] || die "install directory cannot be empty"

os=$(uname -s 2>/dev/null || true)
arch=$(uname -m 2>/dev/null || true)

case "$os/$arch" in
    Linux/x86_64 | Linux/amd64)
        target=x86_64-unknown-linux-gnu
        archive_ext=tar.gz
        ;;
    Darwin/arm64 | Darwin/aarch64)
        target=aarch64-apple-darwin
        archive_ext=tar.gz
        ;;
    Darwin/x86_64)
        target=x86_64-apple-darwin
        archive_ext=tar.gz
        ;;
    Linux/aarch64 | Linux/arm64)
        die "Linux arm64 binaries are not published yet; build from source: https://github.com/dumbramkrishna/pu-get#build-from-source"
        ;;
    MINGW* | MSYS* | CYGWIN*)
        die "Windows is supported by install.ps1: https://github.com/dumbramkrishna/pu-get/blob/main/install.ps1"
        ;;
    *)
        die "unsupported platform: ${os:-unknown}/${arch:-unknown}. Build from source instead."
        ;;
esac

if [ -z "$VERSION" ]; then
    note "Looking up the newest pu-get release..."
    releases=$(fetch_stdout "$API_BASE/repos/$REPO/releases?per_page=1") ||
        die "could not query GitHub releases; pass --version v0.0.1 to skip the lookup"
    VERSION=$(printf '%s\n' "$releases" |
        sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' |
        head -n 1)
    [ -n "$VERSION" ] || die "no published release found; pass --version v0.0.1"
fi

case "$VERSION" in
    v*) ;;
    *) VERSION="v$VERSION" ;;
esac

case "$VERSION" in
    *[!A-Za-z0-9._-]*) die "invalid version: $VERSION" ;;
esac

asset="pu-get-${VERSION}-${target}.${archive_ext}"
release_url="$DOWNLOAD_BASE/$REPO/releases/download/$VERSION/$asset"
checksum_url="$DOWNLOAD_BASE/$REPO/releases/download/$VERSION/SHA256SUMS.txt"
destination="$INSTALL_DIR/pu-get"

if [ "$DRY_RUN" -eq 1 ]; then
    note "Would install pu-get ${VERSION#v}"
    note "  platform:  $target"
    note "  archive:   $release_url"
    note "  checksum:  $checksum_url"
    note "  install:   $destination"
    exit 0
fi

if [ -f "$destination" ] && [ "$FORCE" -eq 0 ]; then
    installed=$("$destination" --version 2>/dev/null || true)
    if [ "$installed" = "pu-get ${VERSION#v}" ]; then
        note "pu-get ${VERSION#v} is already installed at $destination"
        note "Use --force to reinstall."
        exit 0
    fi
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/pu-get-install.XXXXXX") ||
    die "could not create a temporary directory"
cleanup() {
    rm -rf "$tmp_dir"
}
trap cleanup EXIT HUP INT TERM

note "Downloading pu-get ${VERSION#v} for ${target}..."
fetch_file "$release_url" "$tmp_dir/$asset" ||
    die "download failed: $release_url"

note "Verifying SHA-256 checksum..."
sums=$(fetch_stdout "$checksum_url") ||
    die "could not download SHA256SUMS.txt for $VERSION"
expected=$(printf '%s\n' "$sums" |
    awk -v asset="$asset" '$2 == asset || $2 == ("*" asset) { print $1; exit }')
[ -n "$expected" ] || die "SHA256SUMS.txt does not contain $asset"
expected=$(printf '%s' "$expected" | tr 'A-F' 'a-f')

if have sha256sum; then
    actual=$(sha256sum "$tmp_dir/$asset" | awk '{ print $1 }')
elif have shasum; then
    actual=$(shasum -a 256 "$tmp_dir/$asset" | awk '{ print $1 }')
elif have openssl; then
    actual=$(openssl dgst -sha256 "$tmp_dir/$asset" | awk '{ print $NF }')
else
    die "sha256sum, shasum or openssl is required to verify the download"
fi
actual=$(printf '%s' "$actual" | tr 'A-F' 'a-f')

[ "$actual" = "$expected" ] ||
    die "checksum mismatch for $asset (expected $expected, got $actual)"

tar -xzf "$tmp_dir/$asset" -C "$tmp_dir" ||
    die "could not extract $asset"
unpacked="$tmp_dir/${asset%.tar.gz}/pu-get"
[ -f "$unpacked" ] || die "archive did not contain a pu-get binary"

if ! "$unpacked" --version >/dev/null 2>&1; then
    die "the downloaded binary failed to run on this system"
fi

mkdir -p "$INSTALL_DIR" ||
    die "could not create $INSTALL_DIR (check permissions)"
if [ -e "$destination" ] && [ ! -f "$destination" ]; then
    die "$destination exists and is not a regular file"
fi

if have install; then
    install -m 0755 "$unpacked" "$destination" ||
        die "could not install to $destination (check permissions)"
else
    cp "$unpacked" "$destination" ||
        die "could not install to $destination (check permissions)"
    chmod 0755 "$destination" ||
        die "could not mark $destination executable"
fi

printf '\nInstalled %s to %s\n' "$("$destination" --version)" "$destination"

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        printf '\n%s is not in your PATH. Add it to your shell profile:\n' "$INSTALL_DIR"
        printf '  export PATH="%s:$PATH"\n' "$INSTALL_DIR"
        printf 'Then restart the shell and run: pu-get --version\n'
        ;;
esac
