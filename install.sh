#!/usr/bin/env bash
# lean installer — Linux / macOS
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/naiih001/lean/main/install.sh | bash
#   LEAN_VERSION=v0.6.1 curl -fsSL ... | bash
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
  LEAN_VERSION   tag to install, e.g. v0.6.1 (default: latest release)
  LEAN_INSTALL_DIR  directory to install to (default: \$HOME/.local/bin)

Options:
  --help         show this help
  --dir DIR      override install dir (or --dir=DIR)
  --version VER  override version (or --version=VER)

Examples:
  curl -fsSL https://raw.githubusercontent.com/naiih001/lean/main/install.sh | bash
  LEAN_VERSION=v0.6.1 curl -fsSL ... | bash
  ./install.sh --dir /usr/local/bin --version v0.6.1
EOF
}

need_arg() {
  # $1 = flag, $2 = possibly-missing value
  if [[ $# -lt 2 || -z "${2:-}" || "$2" == -* ]]; then
    echo "error: $1 requires a value" >&2
    usage
    exit 1
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --dir) need_arg "$1" "${2:-}"; INSTALL_DIR="$2"; shift 2 ;;
    --dir=*) INSTALL_DIR="${1#--dir=}"; [[ -n "$INSTALL_DIR" ]] || { echo "error: --dir requires a value" >&2; exit 1; }; shift ;;
    --version) need_arg "$1" "${2:-}"; VERSION="$2"; shift 2 ;;
    --version=*) VERSION="${1#--version=}"; [[ -n "$VERSION" ]] || { echo "error: --version requires a value" >&2; exit 1; }; shift ;;
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

detect_raw_arch() {
  case "$(uname -m)" in
    x86_64|amd64) echo "x86_64" ;;
    arm64|aarch64) echo "arm64" ;;
    *) echo "unsupported arch: $(uname -m) — lean ships x86_64 binaries only" >&2; exit 1 ;;
  esac
}

OS="$(detect_os)"
RAW_ARCH="$(detect_raw_arch)"
# Only x86_64 assets exist today.
ARCH="x86_64"
if [[ "$RAW_ARCH" == "arm64" ]]; then
  if [[ "$OS" == "macos" ]]; then
    echo "note: Apple Silicon detected — installing the x86_64 binary (requires Rosetta 2)." >&2
    echo "  For a native build instead: cargo build --release (Rust 1.78+)." >&2
  else
    echo "error: ARM64 Linux detected — lean ships x86_64 binaries only." >&2
    echo "  Build from source: cargo build --release (Rust 1.78+)." >&2
    exit 1
  fi
fi
ASSET="lean-${OS}-${ARCH}.tar.gz"

resolve_version() {
  if [[ -n "$VERSION" ]]; then
    echo "$VERSION"
    return
  fi
  if ! command -v curl >/dev/null 2>&1; then
    echo "error: could not resolve latest version — curl is required for auto-detection." >&2
    echo "  Install curl, or pin a version: LEAN_VERSION=v0.6.1 or --version v0.6.1." >&2
    exit 1
  fi
  # 1. Try GitHub API /releases/latest (with jq)
  if command -v jq >/dev/null 2>&1; then
    local v
    v="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null | jq -r .tag_name 2>/dev/null || true)"
    if [[ -n "$v" && "$v" != "null" ]]; then echo "$v"; return; fi
  fi
  # 2. Fallback: follow redirect on /releases/latest
  local loc
  loc="$(curl -fsSI "https://github.com/${REPO}/releases/latest" 2>/dev/null | tr -d '\r' | awk -F': ' '/^Location:/{print $2}' | tail -1 || true)"
  if [[ "$loc" =~ /tag/(v[^/]+) ]]; then
    echo "${BASH_REMATCH[1]}"; return
  fi
  # 3. Fallback: GitHub API /tags (latest tag)
  if command -v jq >/dev/null 2>&1; then
    local tag
    tag="$(curl -fsSL "https://api.github.com/repos/${REPO}/tags?per_page=1" 2>/dev/null | jq -r '.[0].name' 2>/dev/null || true)"
    if [[ -n "$tag" && "$tag" != "null" ]]; then echo "$tag"; return; fi
  fi
  echo "error: could not resolve latest version from GitHub" >&2
  echo "  No releases or tags found for ${REPO}." >&2
  echo "  Set LEAN_VERSION=v0.6.1 or use --version v0.6.1 and retry." >&2
  exit 1
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

verify_sha256() {
  # $1 = file to check, $2 = .sha256 file (format: "<hash>  <name>", name may carry a legacy path prefix)
  local file="$1" sha_file="$2"
  local expected actual
  expected="$(awk '{print $1}' "$sha_file" | head -1 | tr -d '\r')"
  if [[ -z "$expected" ]]; then
    echo "warning: could not parse checksum from $sha_file — skipping verification" >&2
    return 0
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$file" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$file" | awk '{print $1}')"
  else
    echo "warning: no sha256sum/shasum found — checksum NOT verified" >&2
    return 0
  fi
  if [[ "$(echo "$expected" | tr '[:upper:]' '[:lower:]')" != "$(echo "$actual" | tr '[:upper:]' '[:lower:]')" ]]; then
    echo "error: sha256 mismatch for $ASSET" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual" >&2
    echo "  The download may be corrupt — retry, or verify manually from the Releases page." >&2
    exit 1
  fi
  echo "  sha256 ok"
}

download_failed() {
  echo "error: download failed: $URL" >&2
  echo "  Check the version tag exists and the asset was published:" >&2
  echo "  https://github.com/${REPO}/releases/tag/${VERSION}" >&2
  echo "  (proxies/firewalls may also block github.com — retry with curl -v for detail)." >&2
  exit 1
}

if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$URL" -o "$TMPDIR/$ASSET" || download_failed
  if curl -fsSL "$SHA_URL" -o "$TMPDIR/$ASSET.sha256" 2>/dev/null; then
    echo "→ verifying sha256"
    verify_sha256 "$TMPDIR/$ASSET" "$TMPDIR/$ASSET.sha256"
  else
    echo "warning: no .sha256 published for ${VERSION} — skipping verification" >&2
  fi
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$TMPDIR/$ASSET" "$URL" || download_failed
  if wget -qO "$TMPDIR/$ASSET.sha256" "$SHA_URL" 2>/dev/null; then
    echo "→ verifying sha256"
    verify_sha256 "$TMPDIR/$ASSET" "$TMPDIR/$ASSET.sha256"
  else
    echo "warning: no .sha256 published for ${VERSION} — skipping verification" >&2
  fi
else
  echo "error: need curl or wget to download" >&2; exit 1
fi

echo "→ extracting to ${TMPDIR}"
tar -xzf "$TMPDIR/$ASSET" -C "$TMPDIR"

# Locate the binary (top-level, or nested if packaging changes)
BIN_SRC=""
if [[ -f "$TMPDIR/lean" ]]; then
  BIN_SRC="$TMPDIR/lean"
else
  BIN_SRC="$(find "$TMPDIR" -maxdepth 3 -type f -name lean -print -quit 2>/dev/null || true)"
fi
if [[ -z "$BIN_SRC" ]]; then
  echo "error: archive missing lean binary" >&2; ls -R "$TMPDIR" >&2; exit 1
fi

mkdir -p "$INSTALL_DIR"
if command -v install >/dev/null 2>&1; then
  install -m 755 "$BIN_SRC" "$INSTALL_DIR/$BIN_NAME" 2>/dev/null || {
    echo "error: cannot write to $INSTALL_DIR (permission denied?)" >&2
    echo "  Retry with sudo, or pick another dir: ./install.sh --dir ~/.local/bin" >&2
    exit 1
  }
else
  cp "$BIN_SRC" "$INSTALL_DIR/$BIN_NAME" || {
    echo "error: cannot write to $INSTALL_DIR (permission denied?)" >&2
    echo "  Retry with sudo, or pick another dir: ./install.sh --dir ~/.local/bin" >&2
    exit 1
  }
  chmod 755 "$INSTALL_DIR/$BIN_NAME"
fi

# macOS Gatekeeper: clear quarantine flag, best-effort
if [[ "$OS" == "macos" ]] && command -v xattr >/dev/null 2>&1; then
  xattr -d com.apple.quarantine "$INSTALL_DIR/$BIN_NAME" 2>/dev/null || true
fi

echo "✓ installed $INSTALL_DIR/$BIN_NAME"
HELLOUT="$("$INSTALL_DIR/$BIN_NAME" --help 2>&1 || true)"
echo "  version: $(echo "$HELLOUT" | head -1)"
if ! echo "$HELLOUT" | head -1 | grep -qi "lean"; then
  if echo "$HELLOUT" | grep -qi "libssl\|shared.*library\|cannot open"; then
    echo "  error: the binary failed to start — missing system libraries." >&2
    echo "  Fix (Debian/Ubuntu): sudo apt-get install -y libssl3 ca-certificates" >&2
  else
    echo "  warning: binary sanity check failed — run '$INSTALL_DIR/$BIN_NAME --help' for detail." >&2
  fi
fi

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "  note: $INSTALL_DIR is not on PATH — add it: export PATH=\"\$PATH:$INSTALL_DIR\" (then restart your shell)" ;;
esac

echo "→ run: lean --help"
echo "  config: set OPENAI_API_KEY and run lean in your project"
