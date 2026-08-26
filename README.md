# adaptive-interaction

![adaptive-interaction](docs/assets/hero.png)

跨 AI、跨 Agent Host 的「能力感知型自適應互動平台」。任何 AI——只要能執行
CLI、呼叫 HTTP API、使用 function/tool calling，或由人類透過桌面 UI 代理——
都能透過同一套 Rust Runtime 探索受器（receptors）、動器（actuators）與工具，
在 deterministic 安全政策的限界下觀察、規劃、行動、驗證並調適。**不使用 MCP。**

```
Discover → Observe → Interpret → Plan → Authorize → Act → Verify → Adapt
```

「不介入」是合法決策；`accepted`（已排入佇列）永遠不等於 `completed`。

## 📚 文件

| 文件 | 內容 |
|---|---|
| **[安裝與部署（ELI5）](docs/INSTALL.md)** | 白話解釋這是什麼＋三步驟安裝＋部署拓撲＋常見問題 |
| **[特點與能力](docs/FEATURES.md)** | 核心循環、四種跨 AI 接入等級、安全管家決策流程（mermaid 圖解） |
| **[人類使用手冊](docs/USER-GUIDE.md)** | Session／同意／計畫／配方／政策／緊急停止的日常操作 |
| **[桌面控制中心指南](docs/DESKTOP-GUIDE.md)** | 視覺化工具各頁面說明＋收據狀態機圖解 |
| **[驗收證據](docs/acceptance-evidence.md)** | 真 daemon＋真 CLI 的端到端驗收紀錄 |
| **[AI 用 Skill](skills/orchestrate-adaptive-interaction/SKILL.md)** | 給 AI 讀的操作規範（人類不用讀） |

## 快速開始

```bash
# 建置並啟動 daemon（HTTP API 綁 127.0.0.1:8787）
cargo run -p interaction-cli -- serve

# 另一個終端：完整閉環
interact-ai session start --label demo
interact-ai capabilities --json
interact-ai receptors push task.lifecycle --fact event=task.completed
interact-ai plan --intent celebration --candidate conversation --min-channels 1 --max-channels 1
interact-ai simulate <plan-id>
interact-ai execute <plan-id>
interact-ai actions show <action-id>
interact-ai verify <action-id>
interact-ai emergency-stop        # 隨時可用，不經任何佇列
```

桌面控制中心（Tauri 2）：

```bash
cd apps/interaction-desktop && pnpm install && pnpm tauri dev
```

## 架構

| 層 | crate / 目錄 | 職責 |
|---|---|---|
| 領域模型 | `crates/interaction-core` | manifests、observation（facts/inferences 分離）、bounded action、receipt 狀態機、policy/consent、traits |
| 安全 | `crates/interaction-policy` | deterministic governor：min() 限界鏈、quiet hours、consent、預算、冷卻、pattern 限界 |
| 配方 | `crates/interaction-recipe` | YAML/JSON 模型＋驗證、條件 DSL、多受器融合、觸發評估（可解釋） |
| 儲存 | `crates/interaction-storage` | SQLite：receipts/plans/sessions/observations/audit |
| 註冊表 | `crates/interaction-registry` | 動態能力註冊、健康、availability、snapshot |
| 事件 | `crates/interaction-events` | bounded bus＋Last-Event-ID 重播 |
| Runtime | `crates/interaction-runtime` | orchestrator（可解釋效用）、executor、sessions、recipes 自主迴圈、緊急停止、watchdog、File=Truth 設定 |
| 工具介面 | `crates/interaction-tool-schema` | Canonical Tool Manifest → OpenAI/Anthropic/Gemini/OpenAPI/JSON-Schema |
| API | `crates/interaction-api` | axum HTTP＋SSE，token 驗證 |
| CLI | `crates/interaction-cli` | `interact-ai`（client＋daemon） |
| Adapter SDK | `crates/interaction-adapter-sdk` | manifest builders、driver receipt 協定、merge |
| 內建 adapters | `adapters/builtin` | push 受器、system.time、conversation/web-ui/log/notification/webhook/mock |
| 桌面 | `apps/interaction-desktop` | Tauri 2＋React：總覽/受器/動器/工具/配方/政策/時間軸＋緊急停止 |
| Skill | `skills/orchestrate-adaptive-interaction` | 跨 AI Agent Skill（SKILL.md＋references＋scripts） |

CLI、HTTP API 與 Tauri 共用**同一套** runtime application services；桌面 UI 是
可選的人類控制中心，不是 AI 使用 Runtime 的必要條件。

## 安全模型（Rust 強制，非提示詞）

- 有效輸出 = min(AI 建議, 使用者偏好, session 限制, 裝置安全上限, 剩餘預算)
- 實體／外部寫入動器與敏感受器（攝影機等級）預設關閉且需 session consent
- 高風險（`riskClass >= high`）需人類明確批准；緊急停止永遠不需批准
- pattern 一律 TTL lease＋watchdog；`repeat: forever` 會被正規化為有界租約
- crash/restart 後不自動恢復高風險輸出；未完成動作標記為 `uncertain`
- 緊急停止：CLI／API／桌面按鈕三路可觸發，取消一切、停止 driver、撤回同意、
  寫入 audit，且不自動恢復

## 測試

```bash
cargo fmt --check && cargo clippy --workspace --all-targets && cargo test --workspace
cd apps/interaction-desktop && pnpm typecheck && pnpm build
```

## 授權

MIT。概念參考 [immersive-vibration-response-skill](https://github.com/ra1nyxin/immersive-vibration-response-skill)
（MIT；未複用其程式碼，僅吸收非同步效果／pattern／cancel 等概念並加上 TTL、
lease、watchdog 與 deterministic policy 改進）。未使用任何
tentacle-monster-roleplay-esp32 受限授權程式碼。
