#!/usr/bin/env bash
# lean installer — Linux / macOS
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/naiih001/lean/main/install.sh | bash
#   LEAN_VERSION=v0.2.0 curl -fsSL ... | bash
#   ./install.sh --help
set -euo pipefail

REPO="naiih001/lean"
BIN_NAME="lean"
INSTALL_DIR="${LEAN_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${LEAN_VERSION:-}"

usage() {
  cat <<EOF
lean installer (linux/macos)

Env:
  LEAN_VERSION   tag to install, e.g. v0.2.0 (default: latest release)
  LEAN_INSTALL_DIR  directory to install to (default: \$HOME/.local/bin)

Options:
  --help         show this help
  --dir DIR      override install dir
  --version VER  override version

Examples:
  curl -fsSL https://raw.githubusercontent.com/naiih001/lean/main/install.sh | bash
  LEAN_VERSION=v0.2.0 curl -fsSL ... | bash
  ./install.sh --dir /usr/local/bin --version v0.2.0
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --dir) INSTALL_DIR="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; usage; exit 1 ;;
  esac
done

detect_os() {
  case "$(uname -s)" in
    Linux*) echo "linux" ;;
    Darwin*) echo "macos" ;;
    *) echo "unsupported os: $(uname -s)" >&2; exit 1 ;;
  esac
}

detect_arch() {
  case "$(uname -m)" in
    x86_64|amd64) echo "x86_64" ;;
    arm64|aarch64) echo "x86_64" ;; # lean currently ships x86_64; arm uses Rosetta / build-from-source
    *) echo "x86_64" ;;
  esac
}

OS="$(detect_os)"
ARCH="$(detect_arch)"
# Only x86_64 assets exist today; map all to x86_64 for now
ASSET="lean-${OS}-${ARCH}.tar.gz"

resolve_version() {
  if [[ -n "$VERSION" ]]; then
    echo "$VERSION"
    return
  fi
  # Try GitHub API (needs jq, but fallback to redirect)
  if command -v curl >/dev/null 2>&1; then
    if command -v jq >/dev/null 2>&1; then
      local v
      v="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null | jq -r .tag_name 2>/dev/null || true)"
      if [[ -n "$v" && "$v" != "null" ]]; then echo "$v"; return; fi
    fi
    # fallback: follow redirect on /releases/latest
    local loc
    loc="$(curl -fsSI "https://github.com/${REPO}/releases/latest" 2>/dev/null | tr -d '\r' | awk -F': ' '/^Location:/{print $2}' | tail -1 || true)"
    if [[ "$loc" =~ /tag/(v[^/]+) ]]; then
      echo "${BASH_REMATCH[1]}"; return
    fi
  fi
  echo "latest tag not found — set LEAN_VERSION=v0.2.0" >&2; exit 1
}

if [[ -z "$VERSION" ]]; then
  VERSION="$(resolve_version)"
fi

if [[ "$VERSION" != v* ]]; then
  VERSION="v${VERSION}"
fi

URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}"
SHA_URL="${URL}.sha256"

TMPDIR="$(mktemp -d)"
cleanup() { rm -rf "$TMPDIR"; }
trap cleanup EXIT

echo "→ lean ${VERSION} (${OS}-${ARCH})"
echo "→ downloading ${URL}"

if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$URL" -o "$TMPDIR/$ASSET"
  # optional sha256
  if curl -fsSL "$SHA_URL" -o "$TMPDIR/$ASSET.sha256" 2>/dev/null; then
    echo "→ verifying sha256"
    # sha files from <=v0.2.1 contain 'dist/' prefix; normalize for verification
    sed -i 's|dist/||g' "$TMPDIR/$ASSET.sha256" 2>/dev/null || true
    if command -v sha256sum >/dev/null 2>&1; then
      (cd "$TMPDIR" && sha256sum -c "$ASSET.sha256" 2>&1 | head -1)
    elif command -v shasum >/dev/null 2>&1; then
      (cd "$TMPDIR" && shasum -a 256 -c "$ASSET.sha256" 2>&1 | head -1)
    fi
  fi
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$TMPDIR/$ASSET" "$URL"
else
  echo "need curl or wget" >&2; exit 1
fi

echo "→ extracting to ${TMPDIR}"
tar -xzf "$TMPDIR/$ASSET" -C "$TMPDIR"

mkdir -p "$INSTALL_DIR"
if [[ -f "$TMPDIR/lean" ]]; then
  install -m 755 "$TMPDIR/lean" "$INSTALL_DIR/$BIN_NAME"
else
  echo "archive missing lean binary" >&2; ls -R "$TMPDIR" >&2; exit 1
fi

echo "✓ installed $INSTALL_DIR/$BIN_NAME"
echo "  version: $("$INSTALL_DIR/$BIN_NAME" --help 2>&1 | head -1 || true)"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "  note: add to PATH: export PATH=\"\$PATH:$INSTALL_DIR\"" ;;
esac

echo "→ run: lean --help"
echo "  config: set OPENAI_API_KEY and run lean in your project"
