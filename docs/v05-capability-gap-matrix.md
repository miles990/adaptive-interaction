# v0.5 Capability Gap Matrix（產品重定位基線）

> 本輪目標:把產品主體拉回三核心 —— **角色生命感與遊戲互動 > 真實硬體閉環 > AI Agent 工作與對話閉環**。
> 本文件是 Phase 0 的誠實基線:不沿用 v0.4「25/25 complete」的完成度敘述;
> 那份矩陣衡量的是治理平台的完成度,不是角色遊戲性與真實硬體的完成度。

## 0. 基線

| 項目 | 值 |
|---|---|
| 基準 commit | `d200e2c` release: v0.4.1(= origin/main,worktree 乾淨) |
| 日期 | 2026-08-28 |
| 環境 | macOS 26.2(Darwin 25.2.0)、Apple M2 Pro(12 核)、rustc 1.94.0、node 24.5.0、pnpm 10.27.0 |

### Phase 0 回歸實測(重定位動工前)

| 套件 | 命令 | 結果 |
|---|---|---|
| Rust fmt | `cargo fmt --check` | 通過(exit 0) |
| Rust clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 通過,0 warnings |
| Rust tests | `cargo test --workspace` | **336 passed / 0 failed / 0 ignored** |
| 前端 typecheck | `pnpm typecheck` | 通過 |
| 前端 unit | `pnpm test`(vitest) | **94 passed / 0 failed**(11 檔) |
| 前端 build | `pnpm build` | 成功 |
| Tauri tests | `cargo test`(src-tauri) | **4 passed / 0 failed** |
| CLI E2E | `./scripts/v03-cli-e2e.sh` | **51 passed / 0 failed** |
| Playwright | `pnpm test:e2e` | **23 passed / 0 failed**(52.7s) |

## 1. 核心一:角色生命感與遊戲互動

| 能力 | 現況(v0.4.1 實際程式碼) | 目標 | 狀態 |
|---|---|---|---|
| 渲染 | 單 canvas 整幀 sprite blit(`SpriteRenderer`,128×128、8 欄 sheet),僅 idle 有 gaze/ear 微動疊加 | 分層組合通道(body/head/gaze/eyes/ears/tail/hair/bubble/audio/position/particles) | **缺** |
| 動畫數 | v2 packs 24 個具名動畫(整幀) | 36 表情 × 進入/保持/小循環/離開 | **缺** |
| Interaction Director | 無;僅 transient 優先階梯(machine.ts)+ 微動作排程(behavior.ts) | Event Normalizer→Attention→Utility→Intent→Scheduler→Mixer | **缺** |
| 自主移動 | 角色從不自行移動(僅使用者拖曳或 `companion.window.adjust`) | 散步/奔跑/急停/跳躍/攀爬/邊緣探頭 | **缺** |
| 游標互動 | 設計上排除原始座標;只有 30s 節流的 pointer-approached + click/drag | 追游標/躲游標/擋游標(本機 16–100ms,不出 WebView) | **缺** |
| 玩具與物理 | 無任何物理/玩具程式碼 | 毛球/紙團/光點/逗貓棒/紙飛機 + 輕量 2D 物理 | **缺** |
| 多角色 | 單一 companion 視窗硬編碼、prefs 單槽 | 多角色/使魔、互相注意、追逐、Roll Call | **缺** |
| 命名/場景/匯入匯出 | 無命名、無場景、pack 固定 5 選項下拉 | 命名、場景切換、角色設定匯入匯出 | **缺** |
| 檔案接取 | 有(drag-drop 確認流程,誠實等待 push 結果) | 保留並加上角色演出 | 部分 |
| 本機反射延遲 | click/drag 同幀反應(<16ms),已達標 | 16–100ms | **已有** |
| 誠實演出 | verified 才有綠勾;truth-state 動畫不在 AI 可播白名單 | 保留 | **已有** |

## 2. 核心二:真實硬體閉環

| 能力 | 現況 | 目標 | 狀態 |
|---|---|---|---|
| HTTP/SSE adapter | 完整(declarative YAML、SSRF 防護、secret://、retry/timeout) | 保留 | **已有** |
| USB Serial | `Transport::Serial` 解析後誠實拒絕;無 serialport 依賴;macOS cu.* 無 stable_id | 可用 adapter(discovery/pairing/reconnect/cancel/idempotency) | **缺** |
| BLE | 同上誠實拒絕;無 btleplug;只有 system_profiler metadata | 可用 adapter(scan/connect/subscribe/restore) | **缺** |
| MQTT | 同上誠實拒絕;無 rumqttc | 可用 adapter(reconnect/QoS/重複訊息) | **缺** |
| 配對儀式 | 只有狀態機模型(ProviderState/TrustLevel),無實際 key exchange | Pairing/verification + nonce/replay 防護 | **缺** |
| 連線監督 | 無 connection supervisor;HTTP 單發 | 持久連線 + reconnect/backoff + 狀態流轉 | **缺** |
| ESP32 參考裝置 | 無韌體、無 BOM、無接線圖;僅 YAML 分類啟發 | 韌體 + BOM + Flash 步驟 + 真機閉環 | **缺** |
| 誠實階梯 | ActionStatus/VerificationVerdict 完整且測試覆蓋 | 新 transport 沿用 | **已有** |
| 裝置導向 UI | CapabilitiesHub 以 receptor/actuator 為中心 | 以裝置為中心、只發現≠已配對≠已測試≠已啟用 | **缺** |

## 3. 核心三:AI Agent 工作與對話閉環

| 能力 | 現況 | 目標 | 狀態 |
|---|---|---|---|
| 真實連接器 | Codex app-server/exec fallback + Claude stream-json,完整 lifecycle/estop/process-tree | 保留 | **已有** |
| Agent 事件→角色 | `mapRuntimeEvent` 只映射 action.*/plan.*/emergency/proactive;agent session 事件無映射 | queued/fetched/working/waiting/blocked/claimed/verified/failed/unknown/cancelled 全數映射為 Behavior Intent | **缺** |
| Session verified | AgentSessionState 止於 ClaimedCompleted;無 session 級驗證步驟 | 獨立驗證後才播 verified 演出 | **缺** |
| Resume | Runtime 支援但 API/CLI/UI 無入口 | 可達的 resume 通路 | **缺** |
| Conversation Provider | 只是 routing role + tool_scope 限縮,無獨立抽象 | 可插拔介面 + 無 Provider 時本機模板降級 | **缺** |
| Approval 對稱性 | codex app-server 可 approve;claude -p 無 approval 通道(誠實回報) | 保留誠實差異、UI 明示 | 部分 |

## 4. 一般人可理解的設定(IA)

| 能力 | 現況 | 目標 | 狀態 |
|---|---|---|---|
| 一級入口 | 一般模式 9 個(SIMPLE_NAV)+ 進階 9 頁 | 5 個:現在/小樞/工作/連接與權限/更多 | **缺** |
| 首次設定 | 7 步精靈 | 3 步 + 漸進式詢問 | **缺** |
| Activity | 側欄整頁 + 右上通知 popover 兩套並存 | 右上 Inbox 為主,不佔一級 | **缺** |
| 設定單一主人 | companion 外觀在 CompanionPage 與 SettingsPage 重複可編;initiative/quietHours 三處可改 | 每項設定唯一 canonical owner | **缺** |
| 風險分級 | Consent 以能力逐項,無 L0–L4 分級呈現 | L0 純呈現不逐次詢問…L4 短效授權 | **缺** |
| Emergency Stop | 頂欄/tray/搜尋/CLI 觸發,SafetyPage 解除 | 保留 | **已有** |

## 5. iPhone Mobile Provider

全部未開始:無 iOS App、無 Bonjour/QR 配對、無 TLS WebSocket provider 通道、無 motion 語意事件、無 BLE gateway。**缺**(Phase 6)。

## 6. 記憶與知識 UI 分層

後端 10 層記憶 + 知識圖譜完整(**已有**);一般 UI 仍暴露 Candidate/Receipt/Context Bundle 技術術語,未分「角色互動記憶/工作記憶/正式知識」三類(**缺**)。

## 7. 本輪不做(明確排除)

- 不新增治理概念、不擴 MCP(不變量)。
- 不引入 3D 遊戲引擎;物理為輕量 2D 自寫。
- iPhone 真機驗收與 ESP32 真機驗收受限於實體環境,能做多少誠實記多少;模擬器結果一律標示為模擬器。

## 8. 既有已知限制(沿承 v0.4,未修改)

CHANGELOG v0.4.0 的 10 項 closing-audit 已知限制全部仍然成立,本文件不重複列出;修掉時同步更新該處與 docs/acceptance-evidence.md。
