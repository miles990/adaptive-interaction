# interact-ai 快速上手（隨 Release 附帶）

這份說明對應你下載的這個版本。完整文件：
https://github.com/miles990/adaptive-interaction/tree/main/docs

## 安裝

```bash
# 一鍵安裝（私有 repo 需先 gh auth login）
bash install.sh                    # 最新版 CLI → ~/.local/bin
bash install.sh --with-skill       # ＋跨 AI skill（給 Claude Code 等 agent 用）
bash install.sh --with-desktop     # ＋桌面控制中心安裝包

# 或手動：解開對應平台的壓縮檔，把 interact-ai 放進 PATH
tar -xzf interact-ai-<版本>-<平台>.tar.gz && mv interact-ai ~/.local/bin/
```

## 60 秒體驗

```bash
interact-ai serve &                # 啟動本機 daemon（127.0.0.1:8787）
interact-ai session start --label demo
interact-ai receptors push task.lifecycle --fact event=task.completed
interact-ai plan --intent celebration --candidate conversation \
    --min-channels 1 --max-channels 1        # 記下 planId
interact-ai simulate <planId>              # 安全管家的裁決預覽
interact-ai execute <planId>               # 執行（記下 actionId）
interact-ai actions show <actionId>        # 收據：accepted ≠ completed
interact-ai outbox                         # AI 實際說了什麼
interact-ai emergency-stop                 # 🔴 隨時全停（--clear 解除）
```

## 桌面控制中心

- macOS：打開 `interaction-control-center_<版本>_aarch64.dmg`，拖進 Applications。
  未簽章：首次啟動右鍵→打開，或 `xattr -dr com.apple.quarantine <app路徑>`
- Linux：`chmod +x interaction-control-center_<版本>_amd64.AppImage` 後直接執行（或裝 .deb）
- Windows：執行 `interaction-control-center_<版本>_x64-setup.exe`

## 自我管理

```bash
interact-ai self version --check   # 檢查新版本
interact-ai self update            # 一鍵更新（含 sha256 驗證）
interact-ai self update --version v0.1.0   # 裝指定版本
interact-ai self install-skill     # 安裝與本版一致的跨 AI skill
interact-ai self install-desktop   # 下載本平台的桌面版
interact-ai self uninstall --yes   # 移除（--purge 連資料一起）
```

## 給 AI 接入

- **Skill 型 agent**：`interact-ai self install-skill`（預設裝到 `~/.claude/skills/`）
- **Tool calling**：`interact-ai tools export --format openai|anthropic|gemini`
- **HTTP host**：`GET http://127.0.0.1:8787/v1/openapi.json`（token 在
  `~/.adaptive-interaction/state/api-token`）
