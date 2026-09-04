# CLAUDE.md — 給在此 repo 工作的 AI

跨 AI「能力感知型自適應互動平台」：Rust runtime＋`interact-ai` CLI＋HTTP API
（127.0.0.1:8787，Bearer token）＋SSE＋Canonical Tool Manifest（OpenAI／Anthropic／
Gemini／OpenAPI／JSON-Schema 產生器）＋跨 AI Skill（`skills/orchestrate-adaptive-interaction`）
＋Tauri 2 控制中心（狀態列常駐＋桌面角色「小樞」）＋v0.5 的真硬體 adapter
（Serial／MQTT／BLE＋ESP32 參考韌體）與 iPhone Mobile Provider。已發布版本 v0.5.1（2026-09-04，tag `v0.5.1`，修補版本；
前一版 v0.5.0 於 2026-09-03）。
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
- **角色呈現層沒有權限主權**：Runtime 只送語意 Character Intent（`docs/character-protocol/README.md` 是唯一契約）；
  truthState／verified 只由 Runtime 決定，adapter／Character Pack 不能改寫安全文字、不能偽造 verified；不支援就誠實降級
  （substituted／reduced／unsupported），安全訊息永遠落到 `system.text`＋可信 host overlay。小樞只是 Reference Adapter，
  Runtime／頁面不得再引用小樞的部位、表情名或 pack id。

## 佈局

- `crates/interaction-core` 領域模型（observation／action／provider／agent／human meta）
- `crates/interaction-{runtime,registry,policy,recipe,events,storage}` 執行核心
- `crates/interaction-{api,cli,tool-schema,adapter-sdk}` 對外介面
- `crates/interaction-adapter-declarative` YAML→HTTP/SSE／Serial／MQTT／BLE 宣告式裝置 adapter
  （SSRF 防護、secret://、`protocol.rs` 裝置線協定 v1：hello 身分＋配對＋cmd/ack＋dedupe）
- `crates/interaction-runtime/src/mobile.rs` iPhone Mobile Provider（TLS wss、配對、每機 token）
- `crates/interaction-character` Character Presentation Protocol 1.0（純函式；schema golden `schemas/character-protocol.schema.json`）；
  `crates/interaction-runtime/src/character.rs` CharacterHub＋真相投影；`crates/interaction-api/src/character_ws.rs` 外部 adapter WebSocket
- `apps/interaction-desktop/src/character/` TS 鏡射＋in-process gateway＋adapters（`shu`／`sprite`／`text`）；
  `public/characters/` 內建 manifest；`src-tauri/src/{host_safety,character_store,character_bridge}.rs` 可信 overlay／匯入／IPC；
  `docs/character-protocol/` 契約、adapter 撰寫指南、reference adapter 導覽；`examples/character-adapters/` 外部 WebSocket fixture
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
interact-ai character status|instances|adapters add --name X --manifest m.json   # 角色協定；安全 intent 不可手動送
./scripts/release-prepare.sh X.Y.Z         # 發布 1/3：版本號＋CHANGELOG＋golden／codegen（不 commit）
./scripts/release-verify.sh X.Y.Z          # 發布 2/3：關卡（worktree／版本／CHANGELOG／secrets／drift／CI）
./scripts/release-tag.sh X.Y.Z --push      # 發布 3/3：從已驗證 commit 打 annotated tag 並推送
./firmware/esp32-companion/compile.sh [--ble] # ESP32 韌體 arduino-cli 編譯檢查（非真機驗收）
cd apps/interaction-desktop && pnpm perf   # 角色效能量測（headless Chromium；文件引用的數字必須由它產生）
# iOS：export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer 再 xcrun swiftc -typecheck（見 apps/interaction-ios/README.md）
```

## 工作規則

- 不自行 push／發布／部署／開 PR，除非使用者要求；發布一律走 `release-prepare.sh → release-verify.sh → release-tag.sh`（v0.6.0 起拆開；`release.sh` 只印流程）。
- 交付前跑全套測試並回報**實際數字**，不寫「全部通過」了事；未完成項列明原因。
- 大改動用 `.claude/workflows/adversarial-review-adaptive-interaction.js` 跑對抗審查
  （find→independent verify），確認的缺陷修掉或誠實記為已知限制。
- 已知限制記錄在 `CHANGELOG.md` 與 `docs/acceptance-evidence.md`，修掉時同步更新。
- Skill 更新後用 `interact-ai self install-skill` 重裝到各 agent home。
- 本機 `target/` 約 30 GB，磁碟接近滿時 build 會以 ENOSPC 中斷；可安全刪除 `target/debug/incremental`
  （純快取），不要動 `deps/`。Apple Silicon 無 Rosetta：arduino-cli 內建 ctags 跑不起來，`compile.sh` 會自動改用
  `firmware/esp32-companion/tools/ctags-shim`（需 `brew install universal-ctags`）。
- 模擬器／fixture／程序內 client 的結果一律標示「模擬器」；ESP32 真板驗收目前為零；iPhone 只有
  `docs/releases/v0.5.0-iphone-device-evidence.md` 逐列標示的真機證據，其餘列不得寫成已驗收。
