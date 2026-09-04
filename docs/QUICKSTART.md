# interact-ai 快速上手（隨 Release 附帶）

這份說明對應你下載的這個版本。完整文件：
https://github.com/miles990/adaptive-interaction/tree/main/docs

## 安裝

```bash
bash install.sh          # all-in-one 互動選單：CLI／Skill／桌面版／completion 勾選安裝
bash install.sh --all    # 非互動全裝；亦有 --with-skill/--with-desktop/--with-completion/--cli-only

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
interact-ai self update            # 一鍵更新（比對 .sha256；缺校驗檔即中止，不裝未驗證的位元組）
interact-ai self update --version v0.1.0   # 裝指定版本
interact-ai self install-skill     # 跨 AI 裝 skill（偵測 Claude/Codex/Gemini/Copilot…預設全裝）
interact-ai self install-desktop   # 下載本平台的桌面版（先驗 sha256 才交給 OS）
interact-ai self uninstall --yes   # 移除（--purge 連資料一起）
```

> 沒有簽章／公證／SBOM／provenance：`.sha256` 只證明位元組與 Release 一致，不證明來源；
> 桌面安裝包未簽章。Linux aarch64 沒有預編譯檔，需 `cargo install --path crates/interaction-cli`。
> 見 [安裝指南](INSTALL.md#完整性驗證能證明什麼不能證明什麼)。

## 給 AI 接入

- **Skill 型 agent**：`interact-ai self install-skill`（自動偵測所有 agent home、預設全裝；--dest 可指定自訂位置）
- **Tool calling**：`interact-ai tools export --format openai|anthropic|gemini`
- **HTTP host**：`GET http://127.0.0.1:8787/v1/openapi.json`（token 在
  `~/.adaptive-interaction/state/api-token`）
