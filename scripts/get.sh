#!/usr/bin/env bash
# adaptive-interaction release installer (uploaded to每個 release 作為 install.sh)
#
# 用法：
#   bash install.sh                     # 安裝最新版 CLI 到 ~/.local/bin
#   bash install.sh --version v0.1.0    # 安裝指定版本
#   bash install.sh --with-skill        # 順便安裝跨 AI skill（版本一致）
#   bash install.sh --with-desktop      # 順便下載桌面控制中心安裝包
#   bash install.sh --bin-dir /usr/local/bin
#
# 私有 repo：需先 `gh auth login`（腳本會自動用 gh 下載）；
# 公開 repo：無需任何憑證。
set -euo pipefail

REPO="miles990/adaptive-interaction"
VERSION=""
BIN_DIR="${HOME}/.local/bin"
WITH_SKILL=0
WITH_DESKTOP=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --bin-dir) BIN_DIR="$2"; shift 2 ;;
    --with-skill) WITH_SKILL=1; shift ;;
    --with-desktop) WITH_DESKTOP=1; shift ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

case "$(uname -s)/$(uname -m)" in
  Darwin/arm64)  TRIPLE="aarch64-apple-darwin" ;;
  Darwin/x86_64) TRIPLE="x86_64-apple-darwin" ;;
  Linux/x86_64)  TRIPLE="x86_64-unknown-linux-gnu" ;;
  Linux/aarch64) TRIPLE="aarch64-unknown-linux-gnu" ;;
  *) echo "unsupported platform: $(uname -s)/$(uname -m); build from source (cargo install --path crates/interaction-cli)" >&2; exit 1 ;;
esac

have() { command -v "$1" >/dev/null 2>&1; }

fetch_latest_tag() {
  if have gh; then
    gh api "repos/${REPO}/releases/latest" --jq .tag_name 2>/dev/null && return 0
  fi
  curl -fsSL ${GITHUB_TOKEN:+-H "Authorization: Bearer $GITHUB_TOKEN"} \
    "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep -o '"tag_name": *"[^"]*"' | head -1 | cut -d'"' -f4
}

download() { # $1 asset, $2 dest-dir
  if have gh; then
    gh release download "$VERSION" --repo "$REPO" --pattern "$1" --dir "$2" --clobber && return 0
  fi
  curl -fsSL ${GITHUB_TOKEN:+-H "Authorization: Bearer $GITHUB_TOKEN"} \
    -o "$2/$1" "https://github.com/${REPO}/releases/download/${VERSION}/$1"
}

[[ -n "$VERSION" ]] || VERSION="$(fetch_latest_tag)"
[[ -n "$VERSION" ]] || { echo "cannot determine latest version (private repo? run: gh auth login)" >&2; exit 1; }
[[ "$VERSION" == v* ]] || VERSION="v${VERSION}"

ASSET="interact-ai-${VERSION}-${TRIPLE}.tar.gz"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "→ downloading ${ASSET}"
download "$ASSET" "$WORK"
download "${ASSET}.sha256" "$WORK" || echo "  (no checksum published; skipping verification)"

if [[ -f "$WORK/${ASSET}.sha256" ]]; then
  EXPECTED="$(awk '{print $1}' "$WORK/${ASSET}.sha256")"
  if have shasum; then ACTUAL="$(shasum -a 256 "$WORK/$ASSET" | awk '{print $1}')";
  else ACTUAL="$(sha256sum "$WORK/$ASSET" | awk '{print $1}')"; fi
  [[ "$EXPECTED" == "$ACTUAL" ]] || { echo "checksum mismatch!" >&2; exit 1; }
  echo "→ checksum ok"
fi

mkdir -p "$BIN_DIR"
tar -xzf "$WORK/$ASSET" -C "$WORK"
install -m 755 "$WORK/interact-ai" "$BIN_DIR/interact-ai"
echo "→ installed: $BIN_DIR/interact-ai ($("$BIN_DIR/interact-ai" --version))"

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "⚠ $BIN_DIR 不在 PATH；加入 shell rc： export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac

if [[ "$WITH_SKILL" == 1 ]]; then
  "$BIN_DIR/interact-ai" self install-skill
fi
if [[ "$WITH_DESKTOP" == 1 ]]; then
  "$BIN_DIR/interact-ai" self install-desktop --version "$VERSION"
fi

cat <<'EOF'

下一步：
  interact-ai serve                 # 啟動 daemon
  interact-ai session start         # 開始互動 session
  interact-ai self version --check  # 之後檢查更新（self update 一鍵升級）
EOF
