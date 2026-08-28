# CLAUDE.md — 給在此 repo 工作的 AI

跨 AI「能力感知型自適應互動平台」：Rust runtime＋`interact-ai` CLI＋HTTP API
（127.0.0.1:8787，Bearer token）＋SSE＋Canonical Tool Manifest（OpenAI／Anthropic／
Gemini／OpenAPI／JSON-Schema 產生器）＋跨 AI Skill（`skills/orchestrate-adaptive-interaction`）
＋Tauri 2 控制中心（狀態列常駐＋桌面角色「小樞」）＋v0.5 的真硬體 adapter
（Serial／MQTT／BLE＋ESP32 參考韌體）與 iPhone Mobile Provider。已發布版本 v0.4.1；
`main` 上為 v0.5 開發中（未發布）。
架構細節見 `docs/ARCHITECTURE.md`，功能總覽見 `docs/FEATURES.md`。

## 不可違反的不變量

- **嚴禁 MCP**：不做 MCP server／client，不把 MCP 設為依賴或介面。
- **安全由 Rust Policy Governor 確定性強制**——不靠 prompt、不靠隱藏 UI 按鈕。
  有效值 = min(AI 請求, 使用者偏好, session 限制, 裝置安全上限, 剩餘預算)。
- **誠實階梯**：queued≠completed；acknowledged≠completed；completed≠verified；
  inference≠fact；結果未知要標 `uncertain`／Unknown，不得謊稱成功。
- AI 不可授予 consent、不可解除 emergency stop、不可提高後端安全上限。
- 實體／外部副作用動器與敏感受器（麥克風、攝影機）**預設關閉**；
  emergency stop 與高風險能力在重啟後**不得自動恢復**。
- 模擬／dry-run 不得產生外部副作用；不用假資料冒充真實 agent／裝置／執行結果。
- 長時工作必須有 TTL／lease／watchdog／cancel；禁止無界 queue 與 blocking sleep；
  production code 不濫用 `unwrap()`。
- CLI／HTTP API／Tauri 共用同一 application service；核心邏輯不進前端 JS；
  WebView 不直接控制裝置。
- 感測不靜默：啟用中的感測器必須同時反映在 status、事件、tray 與 UI。

## 佈局

- `crates/interaction-core` 領域模型（observation／action／provider／agent／human meta）
- `crates/interaction-{runtime,registry,policy,recipe,events,storage}` 執行核心
- `crates/interaction-{api,cli,tool-schema,adapter-sdk}` 對外介面
- `crates/interaction-adapter-declarative` YAML→HTTP/SSE／Serial／MQTT／BLE 宣告式裝置 adapter
  （SSRF 防護、secret://、`protocol.rs` 裝置線協定 v1：hello 身分＋配對＋cmd/ack＋dedupe）
- `crates/interaction-runtime/src/mobile.rs` iPhone Mobile Provider（TLS wss、配對、每機 token）
- `adapters/{builtin,media}` 內建受器動器＋麥克風感測（feature-gated cpal）
- `apps/interaction-desktop` Tauri 2 控制中心＋小樞（`src/companion/rig/` 執行期分層 rig；
  `scripts/shu/` 有 v2 sprite 產生器、`preview-rig.mjs` 設計稿、`perf-rig.mjs` 效能量測）
- `apps/interaction-ios` SwiftUI iPhone companion app（模擬器驗收；真機未驗）
- `firmware/esp32-companion` ESP32 參考韌體＋BOM／接線／`compile.sh`（arduino-cli 編譯檢查）
- `schemas/` golden schemas（由 release.sh 重生）；`skills/` 跨 AI Skill

## 常用命令

```bash
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                     # Rust 測試
cd apps/interaction-desktop && pnpm typecheck && pnpm test && pnpm build
pnpm test:e2e                              # Playwright（自起真 daemon＋Chromium）
./scripts/v03-cli-e2e.sh                   # CLI 驗收（真 daemon＋mock device）
interact-ai serve                          # daemon；token 在 ~/.adaptive-interaction/state/api-token
./scripts/release.sh vX.Y.Z                # 發布：重生 golden schemas、打 tag、觸發 Release CI
./firmware/esp32-companion/compile.sh [--ble] # ESP32 韌體 arduino-cli 編譯檢查（非真機驗收）
cd apps/interaction-desktop && pnpm perf   # 角色效能量測（headless Chromium；文件引用的數字必須由它產生）
# iOS：export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer 再 xcrun swiftc -typecheck（見 apps/interaction-ios/README.md）
```

## 工作規則

- 不自行 push／發布／部署／開 PR，除非使用者要求；發布一律走 `release.sh`。
- 交付前跑全套測試並回報**實際數字**，不寫「全部通過」了事；未完成項列明原因。
- 大改動用 `.claude/workflows/adversarial-review-adaptive-interaction.js` 跑對抗審查
  （find→independent verify），確認的缺陷修掉或誠實記為已知限制。
- 已知限制記錄在 `CHANGELOG.md` 與 `docs/acceptance-evidence.md`，修掉時同步更新。
- Skill 更新後用 `interact-ai self install-skill` 重裝到各 agent home。
- 本機 `target/` 約 30 GB，磁碟接近滿時 build 會以 ENOSPC 中斷；可安全刪除 `target/debug/incremental`
  （純快取），不要動 `deps/`。Apple Silicon 無 Rosetta：arduino-cli 內建 ctags 跑不起來，`compile.sh` 會自動改用
  `firmware/esp32-companion/tools/ctags-shim`（需 `brew install universal-ctags`）。
- 模擬器／fixture／程序內 client 的結果一律標示「模擬器」；ESP32 與 iPhone 真機驗收目前為零，不得寫成已驗收。
