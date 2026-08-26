#!/usr/bin/env bash
# adaptive-interaction all-in-one installer（隨每個 release 附帶為 install.sh）
#
# 互動模式（預設，TTY 下）：選單勾選要安裝的元件。
# 非互動模式（CI／pipe，或給了任一 --with-*／--all）：照旗標裝。
#
#   bash install.sh                      # 互動選單
#   bash install.sh --all                # 全裝（CLI＋skill＋桌面＋completion）
#   bash install.sh --with-skill --with-completion
#   bash install.sh --version v0.1.0 --bin-dir /usr/local/bin
#
# 公開 repo 免憑證；私有 fork 需 gh auth login 或 GITHUB_TOKEN。
set -euo pipefail

REPO="miles990/adaptive-interaction"
VERSION=""
BIN_DIR="${HOME}/.local/bin"
SEL_CLI=1        # CLI 一律安裝（其他元件都靠它）
SEL_SKILL=1      # 預設全選（all-in-one）；用選單或旗標縮小
SEL_DESKTOP=1
SEL_COMPLETION=1
EXPLICIT=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --bin-dir) BIN_DIR="$2"; shift 2 ;;
    --all) SEL_SKILL=1; SEL_DESKTOP=1; SEL_COMPLETION=1; EXPLICIT=1; shift ;;
    --with-skill) [[ "$EXPLICIT" == 0 ]] && { SEL_DESKTOP=0; SEL_COMPLETION=0; }; SEL_SKILL=1; EXPLICIT=1; shift ;;
    --with-desktop) [[ "$EXPLICIT" == 0 ]] && { SEL_SKILL=0; SEL_COMPLETION=0; }; SEL_DESKTOP=1; EXPLICIT=1; shift ;;
    --with-completion) [[ "$EXPLICIT" == 0 ]] && { SEL_SKILL=0; SEL_DESKTOP=0; }; SEL_COMPLETION=1; EXPLICIT=1; shift ;;
    --cli-only) SEL_SKILL=0; SEL_DESKTOP=0; SEL_COMPLETION=0; EXPLICIT=1; shift ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

# ---------- 互動選單 ----------
mark() { [[ "$1" == 1 ]] && echo "x" || echo " "; }

if [[ "$EXPLICIT" == 0 && -t 0 && -t 1 ]]; then
  # 預設全選；輸入編號可取消不要的元件
  while true; do
    echo ""
    echo "adaptive-interaction all-in-one 安裝 — 預設全選，輸入編號可取消"
    echo "  [x] 1. interact-ai CLI（必裝：runtime／daemon／所有指令）"
    echo "  [$(mark $SEL_SKILL)] 2. 跨 AI Skill → ~/.claude/skills/（給 Claude Code 等 agent）"
    echo "  [$(mark $SEL_DESKTOP)] 3. 桌面控制中心（下載本平台安裝包）"
    echo "  [$(mark $SEL_COMPLETION)] 4. Shell completion（$(basename "${SHELL:-zsh}")）"
    echo ""
    read -r -p "切換 2/3/4，a=全選，n=只裝 CLI，Enter=開始安裝 > " choice
    case "$choice" in
      "") break ;;
      a|A) SEL_SKILL=1; SEL_DESKTOP=1; SEL_COMPLETION=1 ;;
      n|N) SEL_SKILL=0; SEL_DESKTOP=0; SEL_COMPLETION=0 ;;
      2) SEL_SKILL=$((1 - SEL_SKILL)) ;;
      3) SEL_DESKTOP=$((1 - SEL_DESKTOP)) ;;
      4) SEL_COMPLETION=$((1 - SEL_COMPLETION)) ;;
      1) echo "（CLI 是其他元件的基礎，固定安裝）" ;;
      *) echo "？輸入 2、3、4、a 或直接 Enter" ;;
    esac
  done
fi

# ---------- 平台偵測 ----------
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
    gh release download "$VERSION" --repo "$REPO" --pattern "$1" --dir "$2" --clobber 2>/dev/null && return 0
  fi
  curl -fsSL ${GITHUB_TOKEN:+-H "Authorization: Bearer $GITHUB_TOKEN"} \
    -o "$2/$1" "https://github.com/${REPO}/releases/download/${VERSION}/$1"
}

[[ -n "$VERSION" ]] || VERSION="$(fetch_latest_tag)"
[[ -n "$VERSION" ]] || { echo "cannot determine latest version (network? private fork → gh auth login)" >&2; exit 1; }
[[ "$VERSION" == v* ]] || VERSION="v${VERSION}"

# ---------- ① CLI ----------
ASSET="interact-ai-${VERSION}-${TRIPLE}.tar.gz"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "→ [1/4] CLI：下載 ${ASSET}"
download "$ASSET" "$WORK"
if download "${ASSET}.sha256" "$WORK"; then
  EXPECTED="$(awk '{print $1}' "$WORK/${ASSET}.sha256")"
  if have shasum; then ACTUAL="$(shasum -a 256 "$WORK/$ASSET" | awk '{print $1}')";
  else ACTUAL="$(sha256sum "$WORK/$ASSET" | awk '{print $1}')"; fi
  [[ "$EXPECTED" == "$ACTUAL" ]] || { echo "checksum mismatch!" >&2; exit 1; }
  echo "    checksum ok"
else
  echo "    (no checksum published; skipping verification)"
fi

mkdir -p "$BIN_DIR"
tar -xzf "$WORK/$ASSET" -C "$WORK"
install -m 755 "$WORK/interact-ai" "$BIN_DIR/interact-ai"
echo "    installed: $BIN_DIR/interact-ai ($("$BIN_DIR/interact-ai" --version))"

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "    ⚠ $BIN_DIR 不在 PATH；請加入 shell rc： export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac

# ---------- ② Skill ----------
if [[ "$SEL_SKILL" == 1 ]]; then
  echo "→ [2/4] 跨 AI Skill"
  "$BIN_DIR/interact-ai" self install-skill
else
  echo "→ [2/4] Skill：略過（之後可 interact-ai self install-skill）"
fi

# ---------- ③ 桌面控制中心 ----------
if [[ "$SEL_DESKTOP" == 1 ]]; then
  echo "→ [3/4] 桌面控制中心"
  "$BIN_DIR/interact-ai" self install-desktop --version "$VERSION"
else
  echo "→ [3/4] 桌面版：略過（之後可 interact-ai self install-desktop）"
fi

# ---------- ④ Shell completion ----------
if [[ "$SEL_COMPLETION" == 1 ]]; then
  echo "→ [4/4] Shell completion"
  SHELL_NAME="$(basename "${SHELL:-zsh}")"
  case "$SHELL_NAME" in
    zsh)
      COMP_DIR="${HOME}/.local/share/interact-ai/completions"
      mkdir -p "$COMP_DIR"
      "$BIN_DIR/interact-ai" completion zsh > "$COMP_DIR/_interact-ai"
      echo "    寫入 $COMP_DIR/_interact-ai"
      echo "    在 ~/.zshrc 加入： fpath=($COMP_DIR \$fpath); autoload -Uz compinit && compinit"
      ;;
    bash)
      COMP_DIR="${HOME}/.local/share/bash-completion/completions"
      mkdir -p "$COMP_DIR"
      "$BIN_DIR/interact-ai" completion bash > "$COMP_DIR/interact-ai"
      echo "    寫入 $COMP_DIR/interact-ai（bash-completion 會自動載入）"
      ;;
    fish)
      COMP_DIR="${HOME}/.config/fish/completions"
      mkdir -p "$COMP_DIR"
      "$BIN_DIR/interact-ai" completion fish > "$COMP_DIR/interact-ai.fish"
      echo "    寫入 $COMP_DIR/interact-ai.fish"
      ;;
    *) echo "    未知 shell（$SHELL_NAME）；手動： interact-ai completion <shell>" ;;
  esac
else
  echo "→ [4/4] completion：略過（之後可 interact-ai completion <shell>）"
fi

cat <<'EOF'

✔ 安裝完成。下一步：
  interact-ai serve                 # 啟動 daemon
  interact-ai session start         # 開始互動 session
  interact-ai self version --check  # 檢查更新（self update 一鍵升級）
  interact-ai self uninstall --yes  # 移除（--purge 連資料）
EOF
