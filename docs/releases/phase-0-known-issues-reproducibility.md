# 階段 0：已知問題可重現性

> **這份文件是什麼**：把「repo 裡既有的已知限制清單」與「階段 0 本輪真的跑出來的缺陷」放在同一張桌上，
> 逐條標明**現在還成不成立**、**用哪一行命令可以再驗一次**、**本階段該不該動它**。它不是新的限制清單，
> 而是既有清單的**可重現性對帳**：每一列都指得到 `file:line`、測試名稱或證據檔路徑；指不到的一律寫
> 「未核實」「not-run」「not-reproducible-in-repo」，不寫成已驗收。
>
> **來源**（全部以 `78dcda1a3733c97d266ca9b60ad4461c69ca2032` 為準，= `origin/main`，v0.8.0 tag `1fa69b8` 之後只有 docs／test commit）：
>
> - 靜態盤點維度 L（已知限制與相容路徑）：`docs/releases/evidence/2026-09-07-phase-0/static-matrix/dim-L.json`
>   ——71 列 find＋71 筆獨立懷疑者 verdict（其中 21 筆 `refuted:true`）＋completeness critic（17 條 missingRows、16 條 contradictions、14 條 reproCandidates）。
> - 本輪真 agent／fixture／原生 E2E 判讀：`docs/releases/evidence/2026-09-07-phase-0/analysis/e2e-baseline.json`
>   ——11 個情境（E0-01～E0-11）、20 條產品缺陷（D1–D20）、6 條環境阻礙。
> - 既有清單本體：`docs/releases/v0.8.0-known-limitations.md`、`CHANGELOG.md` 的 `[Unreleased]` 與 `[0.8.0]` Known limitations、
>   `docs/acceptance-evidence.md` 末段、`docs/releases/v0.7.0-known-limitations.md`、`docs/aip/deprecation-ledger.md`。
> - 基線測試逐步 exit code 與起訖時間：`docs/releases/evidence/2026-09-07-phase-0/baseline/summary.jsonl`。
>
> **怎麼再產生**：
>
> ```bash
> # 1) 全套基線（逐步落 summary.jsonl，含 fmt/clippy/cargo test/pnpm/Playwright/iOS 模擬器/drills/ESP32 compile）
> PHASE0_OUT=<out> scripts/tests/phase0/baseline.sh
> # 2) 真 agent E2E（會花真實額度；每支腳本自帶隔離 INTERACT_AI_HOME）
> python3 scripts/tests/phase0/agent_smoke.py --agent claude-code --port 19010 --out <out>/claude-smoke
> python3 scripts/tests/phase0/multi_session.py --help ; python3 scripts/tests/phase0/restart_test.py --help
> # 3) 本文件逐列的 grep／cargo／pnpm 命令：見 §2 與 §4 每一列
> # 4) 文件誠實度對帳
> bash scripts/tests/architecture-checks.sh --docs
> ```
>
> **用語（只用這些值，不自創）**：
> 狀態＝`implemented-and-connected`／`implemented-partial`／`defined-only`／`absent`／`confirmed-defect`／`needs-investigation`；
> 證據層級＝`static-inspection`／`unit`／`contract`／`fixture`／`simulator`／`integration`／`browser`／`native-desktop`／`real-agent`／`real-iphone`／`real-hardware`／`not-run`。
> 誠實階梯：queued ≠ completed；acknowledged ≠ completed；claimed ≠ verified；inference ≠ fact。
> **本輪 real-iphone 與 real-hardware 各 0 筆**；fixture／模擬器／真 agent／原生桌面分開標，不互相頂替。

## 0. 讀這份文件之前要知道的四件事

1. **`refuted:true` 的列一律以 `correctedStatus` 為準。** 21 筆改判逐條列在 §3，每一列在 §2 的狀態欄已經是改判後的值。
2. **critic 的 missingRows 不是結論。** 它們在附錄 A，統一標「completeness critic 提出，未逐列核實」——本文件沒有替它們做 find→verify。
3. **「隨全套執行」不等於「逐支隔離驗證」。** 本輪基線在 HEAD `78dcda1` 跑了
   `cargo test --workspace --no-fail-fast`、`cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml`、
   `pnpm typecheck`／`pnpm test`／`pnpm build`／`pnpm test:e2e`、`bash scripts/v03-cli-e2e.sh`、
   `architecture-checks.sh` 的 `--docs`／`--ts`／`--rust`／`--swift`／`--drills`／`--drill-lint`、
   `scripts/tests/ios-simulator.sh`、`firmware/esp32-companion/compile.sh`（含 `--ble`），**每一步 exit 0**
   （逐步命令、起訖時間與 exit code：`docs/releases/evidence/2026-09-07-phase-0/baseline/summary.jsonl`）。
   凡是重現命令屬於這些套件子集的列，本文件只寫「隨全套執行（未逐支隔離）」——那是真的跑過，但**沒有**單獨看那支的紅綠。
   本文件不引用任何「某支測試檔 N 測」這種手抄計數。
4. **證據路徑的歸檔規則。** 引用一律用 repo 相對路徑 `docs/releases/evidence/2026-09-07-phase-0/…`。
   歸檔時：`.log` → `.txt`、`*.stdout`/`*.stderr` → `*.stdout.txt`/`*.stderr.txt`；**名稱含 `home` 的目錄（token、SQLite）一律不歸檔**
   （見 `docs/releases/evidence/2026-09-07-phase-0/artifact-manifest.json`），所以來源資料裡指向 `…/home/state/interaction.db` 的證據
   在 repo 內**查不到**，本文件遇到這種引用會明說。
   `docs/releases/evidence/2026-09-07-phase-0/harness-used/` 是本輪實際使用的 harness 副本；它與現在提交的
   `scripts/tests/phase0/` 只差一行（ROOT 由寫死的絕對路徑改成相對於腳本位置解析），其餘逐字相同。

## 1. 一覽

### 1.1 既有已知限制 71 列的狀態分布（依 `correctedStatus`）

| 狀態 | 列數 | 說明 |
|---|---|---|
| `implemented-and-connected` | 23 | 有正式入口、行為確定（多數是 by-design 的誠實邊界） |
| `implemented-partial` | 30 | 接到正式入口但有明說的缺口 |
| `defined-only` | 7 | 只有型別／schema／文件，沒有可達的正式路徑 |
| `absent` | 3 | 連定義都沒有 |
| `needs-investigation` | 6 | 缺環境或缺追查，現況未定 |
| `confirmed-defect` | 2 | L-02（claude 中斷永不落 cancelled）、L-60（重啟後產品層不揭露「結局未知」） |

（合計 71。分布由 `docs/releases/evidence/2026-09-07-phase-0/static-matrix/dim-L.json` 的 `verdicts[].correctedStatus`（無值時取 `found.rows[].status`）計得。）

### 1.2 本輪對這 71 列做了什麼

| 情形 | 列 |
|---|---|
| 本輪以真 agent／真 daemon 重現 | L-01、L-02、L-03、L-60（API 層） |
| 本輪前提已改變（不再是「零證據」） | L-10（真 Claude Code／Codex 已跑完 E0-02～E0-10） |
| 本輪撰稿時逐條實跑靜態命令並確認 | L-06、L-13、L-15、L-16、L-24、L-31、L-36、L-40、L-41、L-46、L-48、L-56、L-62、L-63、L-64、L-67（部分） |
| 隨全套測試執行（未逐支隔離） | 其餘以 `cargo test`／`pnpm test`／`pnpm typecheck`／`pnpm test:e2e` 為重現命令的列 |
| 本輪確定沒跑 | L-17、L-19、L-35（native 半邊）、L-39、L-50、L-52、L-53、L-58（走查半邊）、L-28（BLE 半邊）、L-29（真板）、L-14（真機） |

## 2. 已知限制逐列表（L-01 – L-71）

> 讀法：**狀態**與**證據層級**已套用懷疑者改判（§3）；**重現／驗證命令**的第一行是可執行命令，
> 第二行（`本輪…`）是本輪實際做到哪裡。`not-reproducible-in-repo` 代表 repo 內沒有任何方式重現，需要外部環境。
> 「本階段可修？」只允許一種 yes：**阻止驗證的最小修復**；產品行為缺陷一律標後續階段。

| 列 | 來源 | 限制描述 ── 邊界／影響 | 狀態 | 證據層級 | 重現／驗證命令 ── 本輪執行情形 | 本階段可修？ |
|---|---|---|---|---|---|---|
| L-01 | 未逐列標註 | Agent「等待人類輸入」（waiting-for-input）狀態：runtime／CLI／桌面都有這個狀態，但沒有任何 connector 會產生它（gateway 註解自承）　──　**邊界／影響**：正式路徑上唯一能到達 WaitingForInput 的方法是 agent 自己 POST /report；真 Claude Code／Codex 都不會這麼做，所以桌面「需要你回話」那一格在真 agent 上永遠不會亮 | `defined-only` | `static-inspection` | `grep -rn TaskWaitingForInput crates/interaction-agent-gateway/src/ \| grep -v lib.rs  # 期望零命中＝限制仍在`<br>**本輪已重現：E0-09／D11**；撰稿時另跑該 grep，`crates/interaction-agent-gateway/src/` 內只命中 `lib.rs:99` 定義 | 否——後續階段 |
| L-02 | 新發現（dim-L notes 自陳不在任何既有限制清單） | 人類按「暫停／中斷目前工作」對 claude-code session 時，後端應落到 cancelled；目前 claude connector 從不產生 TaskCancelled，SIGINT 後結局落成 unknown（或非零 exit 時 failed）　──　**邊界／影響**：未在任何已知限制清單中揭露；一般模式「取消工作」任務（general-mode-tasks.spec.ts 任務 11）在 fixture 上只用 codex 走通，claude-code 使用者按中斷會看到「結果未知」而不是「已取消」。真 claude 二進位收到 SIGINT 的實際 exit code 未量測（可能 130 → failed） | `confirmed-defect` | `static-inspection`／`fixture` | `grep -c TaskCancelled crates/interaction-agent-gateway/src/claude.rs`（期望 0）；完整真 agent 重現見 §4 D1<br>**本輪已重現：E0-03／D1**（5/5 真 claude；且實測終態是 `failed` 而非本列原述的 `unknown`） | 否——後續階段修復；本階段只做「記錄實際狀態序列」（已完成，見 D1） |
| L-03 | 未逐列標註 | daemon 重啟後，重啟前仍 open 的 agent session 應誠實標為「不知道結局」：記錄改 Expired、發一則 unknown 狀態事件、收割孤兒子程序群　──　**邊界／影響**：by-design：open session 不跨重啟續命；要接續必須新建 session 並帶 resumeProviderSessionId＋同一 workdir（見 L-04）。非 unix 平台孤兒子程序不清理 | `implemented-and-connected` | `unit`／`fixture` | `cargo test -p interaction-runtime --test agents_loop restart_reports_unknown_for_work_that_was_still_open -- --nocapture`<br>**本輪已重現：E0-04**（真 claude／真 codex 各一次 SIGKILL 後重啟）；並隨基線全套 `cargo test --workspace` 執行 | 否——by-design／無需修復 |
| L-04 | v0.5.1（Breaking） | 續開（resume）不得放寬：舊記錄沒有 resolvedWorkdir、workdir 不同、工具集變寬、找不到原授權紀錄一律 PolicyBlocked；升級前建立的 gateway session 不可續開（v0.5.1 Breaking）　──　**邊界／影響**：by-design 邊界；純對話 agent 的 resumeProviderSessionId 不受檢（無害但不一致） | `implemented-partial` | `unit`／`fixture` | `cargo test -p interaction-runtime --test gateway_loop resume_ -- --nocapture`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-05 | v0.5.1（Breaking） | legacy 共享 agent token 不能 interrupt session；session token 只能中斷自己；human token 可中斷任何（v0.5.1 Breaking）　──　**邊界／影響**：by-design 安全收斂；以 state/api-agent-token 的舊 connector 必須改用 INTERACT_AI_SESSION_TOKEN | `implemented-and-connected` | `integration` | `cargo test -p interaction-api --test api_e2e interrupt_requires_session_ownership_or_a_human_token`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-06 | 未逐列標註 | AI 建立 session 時指定模型（SessionSpec.model）：gateway 有欄位，但 HTTP／CLI 建立 payload 沒有 model，runtime 從不設定它　──　**邊界／影響**：型別與 connector 看似支援模型選擇，但 HTTP／CLI／Tauri 正式入口無法傳入；codex app-server 路徑即使加了 payload 欄位也沒有接線 | `defined-only` | `static-inspection` | `grep -n model crates/interaction-runtime/src/gateway.rs crates/interaction-runtime/src/agents.rs`（期望零命中＝runtime 從不設定 model）<br>**本輪已重現**（撰稿時實跑，兩檔零命中） | 否——後續階段 |
| L-07 | 未逐列標註 | intent-only（零工具）session 與主動式對話：codex 無法確定性停用全部工具故拒絕；generativeAgent 只接受 claude-code　──　**邊界／影響**：by-design（誠實拒絕）；主動說話功能實質上只支援 Claude Code | `implemented-partial` | `unit`／`fixture` | `cargo test -p interaction-runtime --test gateway_loop codex_refuses_intent_only_sessions_it_cannot_enforce`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-08 | 未逐列標註 | claude -p 模式沒有互動核可管道：TaskWaitingForConsent／resolve_approval 只對 codex 有效；claude 寫入型 session 靠 --permission-mode acceptEdits　──　**邊界／影響**：by-design；Claude 的「等你允許」狀態永遠不會出現，安全靠 permission-mode 與 Policy Governor | `implemented-partial` | `fixture`／`not-run` | `cargo test -p interaction-runtime --test gateway_loop claude_sessions_never_show_a_consent_prompt_nobody_can_answer`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-09 | v0.5.1 §4.1 #1 | GET /v1/agent-sessions 無分頁；桌面每個 runtime 事件都全量重取（v0.5.1 §4.1 #1）；歷史有界（已關閉 200 筆／30 天）　──　**邊界／影響**：by-design 邊界（有界但無分頁）；多 session 高頻事件時桌面反覆全量 GET | `implemented-partial` | `unit`／`static-inspection` | `cargo test -p interaction-runtime --test agents_loop closed_agent_session_history_is_bounded_and_pruned_from_storage`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——後續階段 |
| L-10 | 未逐列標註 | 真 codex／claude 二進位對接（gateway／AIP／Character Session）在 repo 自動化證據中零執行；所有 runtime／e2e／原生走查都用 fake_claude.sh／fake_codex.sh　──　**邊界／影響**：本機已登入 Claude Code 2.1.263／Codex 0.153.4，但 repo 內沒有任何一列 real-agent 證據；fixture 模擬程度有限（見 contract） | `needs-investigation` | `fixture`／`browser`／`native-desktop`／`not-run` | `cd apps/interaction-desktop && E2E_REAL_AGENTS=1 pnpm test:e2e  # 需 PATH 上真 claude/codex 已登入；會花費真實額度；session 證據測試會 skip`<br>**本輪前提已改變**：真 Claude Code 2.1.263／Codex 0.153.4 已跑完 E0-02～E0-10（不經 `E2E_REAL_AGENTS`，改用 `scripts/tests/phase0/agent_smoke.py` 等） | 本階段只做驗證（非修復） |
| L-11 | v0.5.1 保留 #22 | ContextBundle 撞到條數／bytes 上限時「遇到第一個放不下的就停」而非貪婪打包，只回報 excluded.overCapacity／truncated（v0.5.1 保留 #22）　──　**邊界／影響**：by-design（確定性優先） | `implemented-partial` | `unit` | `cargo test -p interaction-runtime --test memory_loop context_bundle_reports_capacity_truncation`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-12 | 未逐列標註 | 記憶匯出／還原只涵蓋記憶項目（scope memory-items-only）；知識節點、素材、收據、角色互動記憶不在範圍　──　**邊界／影響**：by-design（方案 B）；使用者備份不含知識與角色互動記憶 | `implemented-partial` | `unit` | `cargo test -p interaction-runtime --test memory_loop memory_export_`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——後續階段 |
| L-13 | v0.6.0 recovery matrix | 記憶／知識子系統缺 browser／真視窗覆蓋（v0.6.0 recovery matrix 沿承）　──　**邊界／影響**：browser 覆蓋只有頁面存在／窄寬度層級；沒有記憶 CRUD／匯出／context-bundle 的 e2e 或 native 走查 | `implemented-partial` | `integration`／`browser` | `grep -n '記憶' apps/interaction-desktop/e2e/app.spec.ts apps/interaction-desktop/e2e/narrow.spec.ts`<br>**本輪已重現**（撰稿時實跑 grep：兩支 e2e spec 共 17 行提到「記憶」，皆非 CRUD／匯出／bundle 走查） | 否——後續階段 |
| L-14 | v0.8.0 表「真 iPhone AIP／applied」 | iPhone 真機：AIP 1.0／Character Session／aip.applied 回執／背景重連在真機零執行；唯一真機證據是 v0.5.0 舊線協定路徑（逐列）　──　**邊界／影響**：real-iphone 只限 v0.5.0 那幾列；AIP 路徑、applied 回執、URLSession 背景行為皆為模擬器／fixture | `needs-investigation` | `fixture`／`simulator`／`real-iphone`／`not-run` | `bash scripts/tests/ios-simulator.sh --out /tmp/ios-evidence --expected-count 163  # 模擬器；真機 not-reproducible-in-repo（需 iPhone＋Xcode 26.6 簽章）`<br>**本輪未取得真機證據**（環境阻礙 B1）；fixture 半邊見 E0-08；iOS 模擬器在基線執行（exit 0） | 否——後續階段 |
| L-15 | v0.8.0 表「iOS socket／heartbeat」 | iOS 不送 AIP heartbeat，存活證明靠舊 status 心跳；收到 host heartbeat 只節流回 legacy status　──　**邊界／影響**：by-design 沿承；presence 語意與 AIP 心跳不同層 | `implemented-partial` | `simulator`／`fixture` | `grep -n heartbeat apps/interaction-ios/InteractionCompanion/Services/SessionClient.swift`<br>**本輪已重現**（撰稿時實跑 grep，`SessionClient.swift` 只在節流回 legacy status 的路徑出現 heartbeat） | 否——後續階段 |
| L-16 | v0.8.0 表「iOS socket／heartbeat」 | URLSessionSocket（真 TLS 指紋固定＋URLSession 回呼執行緒）沒有任何測試；背景閘門測試用替身 socket　──　**邊界／影響**：only code review | `implemented-partial` | `simulator`／`not-run` | `grep -rl URLSessionSocket apps/interaction-ios/InteractionCompanionTests  # 期望零命中`<br>**本輪已重現**（撰稿時實跑 `grep -rl URLSessionSocket …InteractionCompanionTests` → 0） | 否——後續階段 |
| L-17 | v0.5.0 真機證據（逐列） | iPhone App 沒有 Bonjour 探索，host 釘在配對當下；桌面換 IP 必須重新配對（真機驗證過的限制）　──　**邊界／影響**：App 端不瀏覽 mDNS；只有 ReconnectDiagnosis 提示重配對 | `implemented-partial` | `real-iphone`／`static-inspection` | `not-reproducible-in-repo（需真機＋桌面換網段）`<br>not-reproducible-in-repo（需真機＋桌面換網段）；**本輪未跑** | 否——後續階段 |
| L-18 | acceptance-evidence 末段 | 手機入站速率上限 30 msg/s，超過即關連線（AIP frame 共用同一窗）；session 端每成員 token bucket 先回 rate-limited　──　**邊界／影響**：by-design；burst 會斷線而非只丟那一則 | `implemented-and-connected` | `fixture` | `cargo test -p interaction-runtime --test character_session_loop flooding_the_session_is_rate_limited`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-19 | v0.6.0 已知限制 | 多裝置同時連線同一 Character Session（兩台手機＋桌面）未有測試涵蓋（v0.6.0 已知限制）　──　**邊界／影響**：無法從程式判定並發成員互動是否正確；只有單手機＋桌面 | `needs-investigation` | `fixture`／`not-run` | `not-reproducible-in-repo（需新增兩個 fixture 手機同時 pair 的測試）`<br>not-reproducible-in-repo（需兩個 fixture 手機同時 pair 的新測試）；**本輪未跑** | 否——後續階段 |
| L-20 | deprecation-ledger | INTERACT_AI_CHARACTER_SESSION=0 回退路徑（503 session-disabled）在取得真機／真板閉環證據前不得移除　──　**邊界／影響**：相容路徑必須保留 | `implemented-and-connected` | `fixture` | `cargo test -p interaction-runtime --test character_session_loop disabled_flag_falls_back_to_the_v051_behaviour`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-21 | v0.7.0 §3.4（:185）＋v0.8.0 表「Sensor journal 舊資料／容量」 | unresolvedStops（感測停止結果未知）跨重啟保留——v0.7.0 §3.4 記為只在記憶體，v0.8.0 以 SQLite sensor journal format 1 持久化　──　**邊界／影響**：殘餘：無法補造 v0.7.0 memory-only 的 unknown；overflow／store failure 保守提醒，不以 TTL 或同 ID 新連線解除 | `implemented-and-connected` | `unit`／`fixture` | `cargo test -p interaction-runtime --test sensors_loop restart_`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-22 | 未逐列標註 | 純 HTTP 宣告式來源的高風險受器停止後持續 stop-unknown（沒有裝置線可送 stop-all）　──　**邊界／影響**：by-design 誠實邊界；HTTP-only 停止路徑的專屬測試未找到（confidence 降） | `implemented-partial` | `static-inspection`／`fixture` | `cargo test -p interaction-runtime --test declarative_session_loop a_silent_device_stays_visible_as_stop_unknown`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-23 | v0.7.0-migration §5.14 | AI（agent／session／adapter token）讀 /v1/status 只拿 unresolvedStopCount，拿不到 unresolvedStops／characterSessionSync 明細　──　**邊界／影響**：by-design（行為變更 v0.7.0 migration §5.14） | `implemented-and-connected` | `integration` | `cargo test -p interaction-api --test api_e2e the_status_an_agent_reads_carries_no_human_layer_records`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-24 | v0.8.0 表「Removed port…」 | UnboundReason::Removed（spec 檔被刪）沒有任何 production 呼叫者；Runtime::note_declarative_removed 只給測試　──　**邊界／影響**：runtime 偵測不到 spec 檔案刪除；下一次啟動才「沒被載入」 | `defined-only` | `fixture` | `grep -rn note_declarative_removed crates/interaction-api crates/interaction-cli apps/interaction-desktop/src-tauri  # 期望零命中`<br>**本輪已重現**（撰稿時實跑 grep → 0 命中） | 否——後續階段 |
| L-25 | 未逐列標註 | rebind 時間預算固定（握手 25 s／總 50 s，不可設定）；rebind_timeout_is_bounded_and_honest 測試本身要跑約 26 s　──　**邊界／影響**：by-design；慢測試 | `implemented-and-connected` | `unit`／`integration` | `cargo test -p interaction-runtime --test declarative_session_loop rebind_timeout_is_bounded_and_honest -- --nocapture`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離，該支約需 26 s） | 否——by-design／無需修復 |
| L-26 | v0.7.0 §3.2（:183） | rebind 等同重新註冊 spec：人類關掉的非 consent 受器由 keptDisabledReceptors 保留；v0.7 §3.2 稱動器 enabled 旗標「沒有測試釘住」——現已有測試　──　**邊界／影響**：文件過期（v0.7 §3.2）；行為本身 by-design | `implemented-and-connected` | `simulator` | `cargo test -p interaction-runtime --test declarative_session_loop rebind_never_restores_external_actuator_enabled_true`<br>隨基線全套 `cargo test --workspace` 執行（exit 0）；撰稿時另 grep 確認測試函式存在於 `declarative_session_loop.rs` | 否——by-design／無需修復 |
| L-27 | v0.7.0-migration §5 | rebind 失敗後不拆已建連線、ProviderState 留 disconnected；transition_provider(id, Available) 對宣告式裝置回 Disconnected（行為變更）　──　**邊界／影響**：by-design；依賴「回傳＝請求狀態」的呼叫端要注意 | `implemented-and-connected` | `simulator`／`fixture` | `cargo test -p interaction-runtime --test providers_loop re_enabling_a_declarative_device_rebinds_without_a_restart`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-28 | v0.8.0 表「MQTT／BLE applied 專屬閉環」 | MQTT／BLE 的 AIP session／applied／cancel／舊 generation 閉環：MQTT 只有 broker 模擬器 rebind 測試；BLE 零測試；applied 閉環只有 serial pty 與 mobile fixture　──　**邊界／影響**：共用 tracker 不等於各 transport 已驗；BLE 只有型別與 transport 層 | `implemented-partial` | `simulator`／`fixture`／`unit`／`not-run` | `cargo test -p interaction-runtime --test mqtt_rebind_loop  # BLE：not-reproducible-in-repo（無 BLE 模擬器）`<br>MQTT 半邊隨基線全套 `cargo test --workspace` 執行（exit 0）；BLE not-reproducible-in-repo，**本輪未跑** | 否——後續階段 |
| L-29 | v0.8.0 表「ESP32 真板／分片／applied」＋deprecation-ledger §3.2 | ESP32 真板驗收為零；參考韌體刻意忽略 aip-frag、不宣告 aip.frag/1 與 aip.applied/1；只有 arduino-cli 編譯檢查＋pty 模擬器　──　**邊界／影響**：needs-environment；模擬器的忽略行為不是韌體的忽略行為 | `implemented-and-connected` | `simulator`／`static-inspection`／`not-run` | `./firmware/esp32-companion/compile.sh && cargo test -p interaction-adapter-declarative --test esp32_sim_conformance`<br>韌體兩組 compile 在基線執行（`firmware-default`／`firmware-ble` 皆 exit 0）；**真板 0 次** | 否——後續階段 |
| L-30 | v0.8.0 表「同步證據」＋deprecation-ledger §6.1／§6.2 | 同步證據：aip/1.0 對 state 沒有 wire 回執（pending-full-state → full-state 只證明「送出成功」）；v0.8.0 新增可協商 aip.applied/1 但 legacy peer 仍 unconfirmed，且 applied 是 peer 自報非已看見螢幕　──　**邊界／影響**：by-design 誠實界線：applied ≠ 物理效果；legacy peer 無回執時不判目前同步 | `implemented-partial` | `simulator`／`fixture`／`unit` | `cargo test -p interaction-runtime --test declarative_session_loop negotiated_state_applied_tracks_snapshot_patch_loss_rebind_and_stale_receipts`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-31 | 未逐列標註 | syncProfile 由協商 role 推導不看逐項 intents；iOS 端零 syncProfile 消費端　──　**邊界／影響**：by-design（守住 interaction-session 零變更）；iPhone 不顯示自己的同步等級 | `implemented-partial` | `simulator`／`unit` | `grep -rn syncProfile apps/interaction-ios/InteractionCompanion  # 期望零命中`<br>**本輪已重現**（撰稿時實跑 grep → 0 命中） | 否——後續階段 |
| L-32 | deprecation-ledger §3.2 | 裝置線 v1.2 分片：出站中途失敗不重送（回 uncertain）；入站分片對未宣告 aip.frag/1 的裝置一律拒（not-advertised）；稽核佇列有界 8 筆、溢位丟最舊但計數　──　**邊界／影響**：by-design（重送可能重複外部副作用） | `implemented-and-connected` | `unit`／`simulator` | `cargo test -p interaction-adapter-declarative --test aip_outbound_atomicity && cargo test -p interaction-adapter-declarative --test aip_link`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-33 | 未逐列標註 | SSE 事件流本身不帶連線世代：接收端決策表規則 0 只能擋飛行中的 GET／resume 回覆，換連線瞬間送達的舊 SSE 事件擋不住　──　**邊界／影響**：by-design 已記錄 | `implemented-partial` | `unit`／`contract` | `cd apps/interaction-desktop && pnpm test -- receive-decision-fixtures`<br>隨基線 `pnpm test` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-34 | 未逐列標註 | typed boundary 三端不要求 patch 帶 payload.hash（規則 14：兩邊都知道才算不符）；iOS hash-mismatch 復原改送 snapshot query 而非 resume　──　**邊界／影響**：by-design 三端一致的裁決 | `implemented-partial` | `contract`／`simulator`／`static-inspection` | `cargo test -p interaction-session --test receive_decisions_from_json`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-35 | v0.7.0 §4.4＋v0.8.0 表「兩個設定 store」 | 偏好（desktop.json）與 Runtime（SQLite）是兩個可恢復的 store，不是跨程序 ACID；陪伴預設恢復 owner 在 Tauri preset_service；真桌面恢復只有 native fixture 走查；persist 失敗回滾以注入 closure 重現，commit_prefs_patch 之外呼叫端（close_decision 等）仍是舊順序（v0.7 §4.4）　──　**邊界／影響**：by-design；close_decision 等呼叫端的舊順序未逐一核對 | `implemented-partial` | `unit`／`native-desktop` | `python3 scripts/tests/tauri-preset-recovery.py --app <built .app> --out /tmp/preset  # 需桌面 session＋AX 權限；單元：cd apps/interaction-desktop && pnpm test -- preset`<br>單元半邊隨 `pnpm test` 執行（exit 0）；native 走查**本輪未跑**（需 HEAD build 的 .app＋AX 權限，見環境阻礙 B3／B4） | 否——by-design／無需修復 |
| L-36 | v0.7.0 §4.5（:200）＋v0.8.0:24 | v0.7 §4.5：preset opId 由 presetId＋ms 決定會同名——v0.8.0 已改 crypto.randomUUID（限制已消失）　──　**邊界／影響**：v0.7 文件該列已由 v0.8.0 明列取代（v0.8.0-known-limitations.md:24 §4.5 UUID） | `implemented-and-connected` | `unit` | `grep -n randomUUID apps/interaction-desktop/src/companion/applyPresetPlan.ts`<br>**本輪已重現**（撰稿時實跑 grep：`applyPresetPlan.ts:73` 預設參數為 `crypto.randomUUID()`） | 否——by-design／無需修復 |
| L-37 | 未逐列標註 | tray「感測停止待確認 N」只在 refresh_tray 週期更新（非即時推送）；「重新連線中」判定只看 provider detail 不看稽核　──　**邊界／影響**：by-design（可能差一次刷新） | `implemented-partial` | `unit` | `cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml host_safety`<br>隨基線 `cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml` 執行（exit 0）；**外部 daemon 模式的事件驅動缺口未被任何測試覆蓋**（改判理由見 §3） | 否——後續階段（原判「無需修復」已被改判推翻，見 §3） |
| L-38 | v0.8.0 表「角色頁面棘輪…」 | 頁面層棘輪：CharacterPreview.tsx／CharacterLibrary.tsx 仍以 entrypoint switch 分流（架構守門測試列為 PENDING）　──　**邊界／影響**：沿承技術債；棘輪防止擴大 | `implemented-partial` | `unit` | `cd apps/interaction-desktop && pnpm test -- architecture-no-entrypoint-switch`<br>隨基線 `pnpm test` 執行（exit 0；未逐支隔離） | 否——後續階段 |
| L-39 | v0.8.0 表「Removed port…」（移除角色包一句） | 「移除已匯入角色套件」沒有 CLI 入口（只有桌面 IPC 與 character_store::remove）　──　**邊界／影響**：觀察項；CLI 使用者無法移除 | `absent` | `static-inspection`／`native-desktop` | `interact-ai character --help  # 期望無 remove 子命令`<br>**本輪未跑** `interact-ai character --help`；撰稿時以 `commands.rs` 靜態確認 `CharacterAction::Adapters` 只有 add／remove adapter | 否——後續階段 |
| L-40 | v0.8.0 表「Removed port、RendererPort／DevicePort」＋deprecation-ledger §1.3 | RendererPort／DevicePort（ports.rs）零實作者，experimental，不得因介面存在宣稱擴充點可用　──　**邊界／影響**：實際 renderer 是 TS CharacterAdapter，裝置在 transport 層 | `defined-only` | `static-inspection` | `grep -rn 'RendererPort\\|DevicePort' crates apps --include='*.rs' \| grep -v ports.rs  # 期望零命中`<br>**本輪已重現**（撰稿時實跑 grep → 0 命中） | 否——後續階段 |
| L-41 | deprecation-ledger §1.1／§1.2 | 相容包裝：migrate_legacy_pack（唯一 #[deprecated]）與 accept_state／accept_state_with_epoch 只剩測試呼叫端　──　**邊界／影響**：移除需跨一個 minor（v0.6.0 標記後已過 v0.7.0／v0.8.0）；目前保留 | `defined-only` | `static-inspection`／`unit` | `grep -rn migrate_legacy_pack crates apps --include='*.rs'`<br>**本輪已重現**（撰稿時實跑 `grep -rn migrate_legacy_pack crates apps --include=*.rs` → 只有 1 行＝定義本身，連測試呼叫端都沒有） | 否——後續階段 |
| L-42 | deprecation-ledger §2.1／§2.2 | 快照容器 format 0→1 遷移（備份 .pre-format-0）；未來 format 不隔離不覆寫（parked）；explicit null 在還原前拒絕（v0.8.0）　──　**邊界／影響**：移除條件無法由 repo 證明（舊檔在使用者機器） | `implemented-and-connected` | `fixture`／`integration` | `cargo test -p interaction-runtime --test character_session_loop snapshot`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-43 | deprecation-ledger §2.3／§2.4 | 8 個舊小樞 pack id 的設定匯入寬容（LEGACY_CHARACTER_IDS）與舊 tab id 折疊表（LEGACY_ANCHORS）保留　──　**邊界／影響**：相容路徑；移除需長支援期 | `implemented-and-connected` | `unit` | `cd apps/interaction-desktop && pnpm test -- settingsTransfer`<br>隨基線 `pnpm test` 執行（exit 0）；**該列引用的測試檔名不存在**，實際覆蓋檔名見 §3 | 否——by-design／無需修復 |
| L-44 | 未逐列標註 | consent maxUses 只在動器派工原子扣減；受器與 tool-operation scope 帶 maxUses 明確拒絕（仍是 TTL）　──　**邊界／影響**：by-design；受器「只這一次」仍是短 TTL | `implemented-partial` | `unit`／`integration` | `cargo test -p interaction-runtime --test consent_one_shot_loop`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——後續階段 |
| L-45 | 未逐列標註 | 首次設定 commit 在「policy 檔已寫、SQLite 未提交」之間被砍沒有 journal 重放（onboarding.partial audit 只揭露）　──　**邊界／影響**：不可化約缺口（by-design 揭露） | `implemented-partial` | `unit` | `cargo test -p interaction-runtime --test human_layer onboarding`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——後續階段 |
| L-46 | 未逐列標註 | 外部角色 adapter：stdio JSON Lines transport 只有規格；外部 WebSocket adapter 的使用者互動不落成 Runtime 觀察（external-adapter-input-not-observed）　──　**邊界／影響**：stdio transport absent；外部互動觀察需新 receptor 命名空間＋consent 設計 | `absent` | `static-inspection`／`unit` | `grep -rn 'input-not-observed' crates/interaction-runtime/src/character.rs`<br>**本輪已重現**（撰稿時實跑 grep：`character.rs:1181,1198` 兩處） | 否——後續階段 |
| L-47 | v0.8.0 表「安裝平台」＋acceptance-evidence 末段 | 安裝平台：未簽章、未公證、無 SBOM／provenance；Linux aarch64 無預建 CLI；直接 binary 啟動不驗 Finder／Gatekeeper　──　**邊界／影響**：by-policy 沿承 | `absent` | `static-inspection` | `bash scripts/tests/release-scripts.sh`<br>`bash scripts/tests/release-scripts.sh`：**本輪已隨 §7 的 `architecture-checks.sh --docs` 執行**（見 §7） | 否——後續階段 |
| L-48 | v0.8.0 表「macOS native UI 不在 CI」 | macOS 原生 AX 走查（tauri-*.py）與演練腳本（scripts/drills）不在 CI；CI 只跑 cargo/pnpm/playwright；drills 由 architecture-checks --drills 本機實跑　──　**邊界／影響**：accepted limitation；重跑需已建 .app＋AX 權限 | `implemented-partial` | `native-desktop`／`static-inspection` | `bash scripts/tests/architecture-checks.sh --drills  # 原生走查需 python3 scripts/tests/tauri-*.py --app <.app>`<br>**本輪已重現**：撰稿時 grep `.github/workflows/` 對 `docs-claims`／`architecture-checks`／`drills` 零命中；`--drills` 只在本機基線跑過（exit 0） | 否——by-design／無需修復 |
| L-49 | 未逐列標註 | architecture-checks 的 swift 群組是 native Swift 純模型（非 iOS 模擬器／真機）；未選群組記 SKIP 不混入 pass　──　**邊界／影響**：by-design | `implemented-and-connected` | `unit` | `bash scripts/tests/architecture-checks.sh --swift  # 需 swiftc（DEVELOPER_DIR）`<br>基線已執行 `architecture-checks.sh --swift`（exit 0） | 否——by-design／無需修復 |
| L-50 | v0.8.0 表「首輪本機發布驗證非0」＋CHANGELOG [0.8.0] Known limitations | v0.8.0 首輪本機 release-verify（main 1fa69b8）workspace gate 非 0 但細項輸出遺失；同 SHA 重跑 0 失敗；原因未定位 not-reproduced　──　**邊界／影響**：無法從程式判定（可能是 flaky 測試，例如 [Unreleased] 提到的 rebind 完成條件競態，但 ci-followup 明言不拿它補造原因） | `needs-investigation` | `not-run` | `bash scripts/release-verify.sh 0.8.0`，stdout/stderr 全存檔（需乾淨 worktree）<br>**本輪未重跑**（基線步驟表沒有 release-verify 這一步）；仍為 not-reproduced | 本階段只做驗證（非修復） |
| L-51 | CHANGELOG [Unreleased] | [Unreleased]：rebind 整合測試完成條件競態（Available 先發布時誤判）——test-only 修正 857f009；受控重現用 pty 模擬器　──　**邊界／影響**：只有測試變更，產品不變；不能據此解釋 L-50 | `implemented-and-connected` | `simulator` | `cargo test -p interaction-runtime --test declarative_session_loop reenable_rebinds_without_restart -- --exact --nocapture`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-52 | v0.8.0 表「效能範圍」＋v0.5.1 觀察項 | 角色效能量測只在 headless Chromium rig/stage（非 WKWebView／真機），三次樣本、60 s 不足排除長期 leak；v0.5.1 觀察項 GC 後保留集合 +523 KB 來源未定位　──　**邊界／影響**：限定測量 | `implemented-partial` | `browser` | `cd apps/interaction-desktop && pnpm perf`<br>**本輪未跑** `pnpm perf` | 否——後續階段 |
| L-53 | v0.8.0 表「真人可用性與解除 emergency stop」 | 真人可用性（非開發者受測者腳本）與人工解除 emergency stop latch：零受測者；AX 腳本秒數不是人類耗時；不代人解除　──　**邊界／影響**：needs-environment | `needs-investigation` | `browser`／`native-desktop`／`not-run` | `not-reproducible-in-repo（需真人受測者）`<br>not-reproducible-in-repo（需真人受測者）；**本輪 0 位受測者** | 否——後續階段 |
| L-54 | v0.8.0 表「真實停止證據」＋deprecation-ledger §4.3 | 真實停止證據 scope 只代表目前可信連線，沒有通用 hardware incarnation；手機自報 false 不提升到物理驗證；同 ID 新連線不得替舊連線背書　──　**邊界／影響**：by-design 誠實界線；需第一個提供 hardware incarnation 的 adapter 才能放寬 | `implemented-partial` | `unit`／`fixture` | `cargo test -p interaction-runtime --test sensor_journal_review`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-55 | v0.5.1（Breaking） | cancel_action：driver 未在 2 s 內確認取消 → 結果 Uncertain（永不寫 cancelled）；內建動器取消一律 Uncertain（v0.5.1 Breaking）　──　**邊界／影響**：by-design 誠實階梯 | `implemented-and-connected` | `unit` | `cargo test -p interaction-runtime cancel`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-56 | v0.5.1 §4.1 #13／#14 | refusedConnections／failedAuths／droppedObservations／unconfirmedActuators 只到事件、audit 與 outbox 文案，桌面 UI 無消費者　──　**邊界／影響**：v0.5.1 §4.1 #13／#14 沿承 | `implemented-partial` | `static-inspection` | `grep -rn 'unconfirmedActuators\\|refusedConnections' apps/interaction-desktop/src --include='*.tsx' \| grep -v test  # 期望零命中`<br>**本輪已重現**（撰稿時實跑 grep → 0 命中） | 否——後續階段 |
| L-57 | v0.5.1（保守方向） | pairing_ever_compared 只存活在 DeviceLink 生命週期：daemon 重啟後第一次握手保守標 pairingUnverified（低估真板）；pairingNotRecompared 無下游消費者　──　**邊界／影響**：by-design 保守方向；根治需 v2 hello 欄位 | `implemented-partial` | `unit` | `cargo test -p interaction-adapter-declarative --test protocol_honesty`<br>隨基線全套 `cargo test --workspace` 執行（exit 0）；「無下游消費者」一句已被改判推翻，見 §3 | 否——後續階段 |
| L-58 | v0.7.0 §4.10 | 桌面 e2e（Playwright）未涵蓋「未解決停止」專屬區塊路由（v0.7 §4.10）；只有 sensors.spec 斷言 unresolved 為空　──　**邊界／影響**：browser 層沒有「有一筆未解決停止 → 人為解除」的走查；native 走查有（fixture） | `implemented-partial` | `unit`／`browser` | `cd apps/interaction-desktop && pnpm test:e2e -- sensors`<br>`pnpm test:e2e` 在基線執行（exit 0）；但**該情境仍無走查**：native 半邊本輪 harness-failed（D19） | 否——後續階段 |
| L-59 | 未逐列標註 | 同一 gateway session 一次只跑一輪：上一輪還在跑時新的 task 不排隊、回 Busy 錯誤，訊息留在信箱且沒有 delivered 戳記（不是 agent 失敗）　──　**邊界／影響**：by-design（禁止無界 queue）；桌面「再交代」在一輪進行中會拿到明確錯誤，使用者需稍後再送——多 session 並行靠多個 session 而非同 session 多輪 | `implemented-and-connected` | `fixture` | `cargo test -p interaction-runtime --test gateway_loop an_undelivered_message_during_a_running_exec_turn_is_not_an_agent_failure`<br>隨基線全套 `cargo test --workspace` 執行（exit 0）；critic 對「全域 by-design」有異議，見附錄 B | 否——by-design／無需修復 |
| L-60 | 未逐列標註 | daemon 重啟後，重啟前仍 open 的 session 在桌面工作卡上顯示的是 record.state=expired（「已到期」muted），「結果不確定」只存在於狀態事件序列 unknown——使用者是否看得到「重啟導致結局未知」未追到 UI　──　**邊界／影響**：無法從程式判定使用者在重啟後看到的文案是否揭露「結局未知」；若卡片只讀 state 不讀 detail，重啟後的工作會被當成一般到期 | `confirmed-defect` | `static-inspection`／`unit` | `cargo test -p interaction-runtime --test agents_loop restart_reports_unknown_for_work_that_was_still_open  # Runtime 層；UI 層需開桌面：建立 session→送 task→重啟 daemon→看工作卡文案`<br>**API 層本輪已重現：E0-04**（record `state=expired`＋detail）；**UI 層本輪未走查**（native harness 被 D19 擋住） | 本階段只做驗證（非修復） |
| L-61 | v0.6.0 §3-9 | AIP session 成員不能靠 consentGrantId 送出需要額外同意的 command：ConsentVerifier 只是純分類 port（DenyAllConsent），gate 對任何非空 consentGrantId 一律 rejected{scope-denied}（v0.6.0 §3-9 fail-closed）　──　**邊界／影響**：by-design 安全收斂（不能被繞過）但「能夠使用」的路徑不存在；需 1.1 approval 流程 | `implemented-partial` | `unit` | `cargo test -p interaction-session --test session_hardening member_messages_may_not_carry_a_consent_grant`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——後續階段 |
| L-62 | v0.6.0 §3-8 | AIP syncClass `timeline`／`realtime` 只有 schema 枚舉與文件邊界，Character Session 只用 `semantic`（v0.6.0 §3-8）　──　**邊界／影響**：只有型別／文件；不得宣稱支援 timeline 同步或 realtime 拖曳 | `defined-only` | `static-inspection` | `grep -rn 'timeline' crates/interaction-session/src crates/interaction-aip/src  # 期望零實作命中`<br>**本輪已重現**（撰稿時實跑：`capability.rs:23-27` 有 `Timeline`／`Realtime` 變體，`session.rs:469` 的 HostOffer 只給 `Semantic`） | 否——後續階段 |
| L-63 | v0.7.0 §2.7 | 局部投影（scope／scopeRevision／scope-only hash）未實作：含 members 的 patch 仍整段重送（v0.7.0 §2.7 deferred）　──　**邊界／影響**：設計只在文件；分片（L-32）是繞過線長上限的替代，不是局部投影 | `defined-only` | `static-inspection` | `grep -rn 'scopeRevision' crates apps --include='*.rs' --include='*.ts' --include='*.swift'  # 期望零命中`<br>**本輪已重現**（撰稿時實跑 grep → 0 命中） | 否——後續階段 |
| L-64 | CHANGELOG [Unreleased] 對帳機制本身 | 已知限制清單本身的可驗證性：CHANGELOG [Unreleased] Known limitations 每條綁一個「它已被修掉」的可執行證據，修掉卻還掛著會被 lint 擋下；文件引用的測試數／檔案存在／evidence-index candidates schema 一併對帳　──　**邊界／影響**：只對帳 [Unreleased]；v0.7.0-known-limitations.md 的過期列（§3.2／§3.4／§4.5，見 L-21／L-26／L-36）不在 lint 範圍，只由 v0.8.0 檔 :24 概括 | `implemented-and-connected` | `unit` | `bash scripts/tests/docs-claims.sh`<br>**本輪已重現**（撰稿時實跑 `bash scripts/tests/docs-claims.sh`：188 passed / 0 failed；完稿時其他階段 0 文件同時落盤後重跑為 212 passed / 0 failed，見 §7） | 否——by-design／無需修復 |
| L-65 | v0.7.0 §2.6（:169） | resume 超過 maxResumePatches（512）的客戶端分支無法從 wire 驗：513 則補丁的回應超過 32 KiB 典型邊界；該分支只有純函式／fixture 產生器覆蓋（v0.7.0 §2.6 partial）　──　**邊界／影響**：by-design 剩餘範圍：權威 host 超限時改回 snapshot，超限分支在真 daemon 上不可達 | `implemented-partial` | `contract`／`unit` | `cargo test -p interaction-session --test receive_decision_fixtures && cd apps/interaction-desktop && pnpm test -- receive-decision-fixtures`<br>隨基線全套 `cargo test --workspace`＋`pnpm test` 執行（皆 exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-66 | v0.7.0 §2.3–2.5／§2.12＋deprecation-ledger §3.3 | 決策表 fixture 設計邊界（v0.7.0 §2.3–2.5、§2.12）：桌面 patch merge 產不出物件時的 reject-invalid 是表外規則無 fixture；TS 不驗 Rust SHA-256 字面值只搬相同／不同關係；fixture 期望由 Rust 產生器算出；audit reason 欄位截 64 chars　──　**邊界／影響**：by-design；守門靠只讀 JSON 的第二消費者＋突變驗證＋三端交答案，不是 wire 層證據 | `implemented-partial` | `contract`／`unit` | `cargo test -p interaction-session --test receive_decisions_from_json`<br>隨基線全套 `cargo test --workspace` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-67 | v0.7.0 §6.3（:223）／§4.7（:202）／§3.2・§3.4・§4.5 | 文件債：adapter-development.md §1 把「加一個角色」與「加一個新 adapter」混成一步（v0.7.0 §6.3）；DESKTOP-GUIDE 裝置條目三種補充句沒有測試要求出現在指南（§4.7）；v0.7.0-known-limitations.md 至少三列在 HEAD 已過期未就地標註（§3.2／§3.4／§4.5）　──　**邊界／影響**：無法從程式判定文件是否誤導（adapter-development.md 本輪未逐字核對） | `needs-investigation` | `static-inspection` | `bash scripts/tests/docs-claims.sh  # 通過不代表這三項已對帳`<br>**本輪已部分重現**：撰稿時確認 `adapter-development.md` §1 已拆成 1a／1b（該子項不成立）；另兩個子項仍成立，見 §6 | 否——後續階段 |
| L-68 | v0.5.1 保留 #12 | 宣告式 HTTP adapter 的逾時分類保守：只有 reqwest is_connect()／is_builder() 算 NotSent，其餘錯誤歸 OutcomeUnknown（uncertain）——少數真的沒送出的請求會被標成不確定（v0.5.1 保留 #12，之後各版未重新核實）　──　**邊界／影響**：by-design 方向安全；測試覆蓋未核對，confidence 低 | `implemented-and-connected` | `static-inspection`／`integration` | `cargo test -p interaction-adapter-declarative --test http_loop`<br>隨基線全套 `cargo test --workspace` 執行（exit 0）；撰稿時另 grep 確認 `http_loop.rs` 兩支具名測試存在 | 否——後續階段 |
| L-69 | deprecation-ledger §3.1 | 裝置線 v1.1 反向承諾「沒送過 capability 的裝置永遠不會收到 aip」：iPhone 側有具名回歸，宣告式裝置側靠 DeviceLink::admit_aip 傳輸層准入，未見同名的單一回歸測試（deprecation-ledger §3.1）　──　**邊界／影響**：無法從測試名判定宣告式側的反向承諾是否被釘住 | `implemented-and-connected` | `unit`／`static-inspection` | `cargo test -p interaction-adapter-declarative --test aip_link  # 通過不證明反向承諾`<br>隨基線全套 `cargo test --workspace` 執行（exit 0）；具名回歸仍缺 | 否——後續階段 |
| L-70 | deprecation-ledger §4.2／§6.3 | 桌面相容路徑：進階診斷 alignment 計數改名無相容層（TS 型別擋）；JSON value IPC 與 raw JSON text IPC（_raw 指令、runtime-event-raw）雙入口並存以保留 hash 原始數字（deprecation-ledger §4.2／§6.3）　──　**邊界／影響**：相容路徑；舊 value IPC 不保證未知 optional number 的精確 hash | `implemented-and-connected` | `unit` | `cd apps/interaction-desktop && pnpm typecheck`<br>隨基線 `pnpm typecheck` 執行（exit 0；未逐支隔離） | 否——by-design／無需修復 |
| L-71 | v0.6.0 recovery matrix §2.7 #11 | agent／session principal 對記憶控制面的直接讀寫拒絕：GET /v1/memory（與 /v1/assets、/v1/knowledge/nodes、/v1/agent-sessions、/v1/audit）由 agent token 讀取須 403 有測試；POST /v1/memory（create）與 POST /v1/memory/context-bundle 由 agent token 呼叫的 403 未見具名斷言（v0.6.0 recovery matrix §2.7 #11 標「需在 v0.6.0 補」）　──　**邊界／影響**：白名單推論為拒絕；POST 路徑的具名回歸未找到，confidence 中 | `implemented-and-connected` | `static-inspection` | `cargo test -p interaction-api --test api_e2e  # 再以 grep '/v1/memory' api_e2e.rs 核對 POST 案例`<br>隨基線全套 `cargo test --workspace` 執行（exit 0）；POST 拒絕的具名斷言仍缺 | 否——後續階段 |

## 3. 懷疑者改判一覽（21 筆 `refuted:true`）

> 來源：`docs/releases/evidence/2026-09-07-phase-0/static-matrix/dim-L.json` 的 `verdicts[]`。
> 這 21 筆是**獨立懷疑者逐條反駁 find 的結果**，§2 的狀態欄已經是改判後的值。
> 格式：原判 X → 改判 Y（理由摘要）。「同判」代表狀態不變、被推翻的是證據等級或引用。

| 列 | 改判 | 理由摘要 |
|---|---|---|
| L-04 | 原判 `implemented-partial` → 改判 `implemented-partial`（證據等級 `unit`／`fixture`／`browser` → `unit`／`fixture`） | resume 功能在 Playwright e2e 零覆蓋（e2e 裡的 resume 命中全是感測／主動式／連線 resume，與本列無關）；唯一前端測試是 jsdom＋`vi.spyOn` 全 mock 的 vitest，依本 repo 用語屬 unit 而非 browser。核心結論與 `agents.rs:453-466` 的 by-design 缺口不變 |
| L-07 | 同判 `implemented-partial` | 結論成立但**強制點引用錯**：codex 拒絕 intent-only 實際發生在 `crates/interaction-runtime/src/gateway.rs:309-314`（`DomainError::Validation`，在組 `SessionSpec` 之前），該列引用的 `codex.rs:145-147` 回的是 `GatewayError::Unavailable` 且正式路徑不可達、零測試覆蓋；`proactive.rs` 的 codex 專屬分支同樣無測試 |
| L-08 | 同判 `implemented-partial`（證據等級加 `not-run`） | 兩支引用測試都到不了 `claude.rs:343-352`：`resolve_approval_as` 先查 approvals map、miss 就回 `NotFound`，而 claude 的 approvals map 恆為空（`TaskWaitingForConsent` 是唯一寫入點，claude 從不產生），該段程式碼實際零覆蓋。「Claude 沒有互動核可管道」這個結論不變 |
| L-13 | 同判 `implemented-partial`（證據等級 `unit`／`browser` → `integration`／`browser`） | 記憶／知識後端測試是「真 SQLite 暫存 home＋完整 Runtime」，屬 integration 不是 unit；browser 半邊只有一支比對 tab 數的 e2e，沒有碰任何 CRUD／匯出／context-bundle API |
| L-16 | 同判 `implemented-partial` | 「only code review」被推翻：`apps/interaction-ios/README.md:665-712` 記載 2026-09-04 曾以**真** `URLSessionSocket` 對真 daemon 在 iOS 模擬器完成配對、能力協商與 touch intent 走查。但那不是可重跑的 repo 測試，`not-run`／無現行自動化覆蓋的結論不變 |
| L-24 | 同判 `defined-only` | 無 production 呼叫者成立（三個入口 grep 零命中）；被推翻的是測試歸屬——該列寫成 `declarative_session_loop.rs`，實際在 `providers_loop.rs`（與唯一呼叫點同檔） |
| L-29 | 原判 `needs-investigation` → 改判 `implemented-and-connected` | 韌體忽略 `aip-frag`、不宣告 `aip.frag/1` 這件事本身已被 static-inspection＋模擬器測試釘死，行為**確定**；懸而未決的只有「真板是否一致」，那是 real-hardware 缺口（該列 `not-run` 已標），不是「連程式行為都沒查清」 |
| L-32 | 同判 `implemented-and-connected`（證據等級 `unit`／`simulator` → `unit`） | 引用的兩支測試用 in-process `MockRawLink`／`YieldingLink`，沒有 spawn `esp32-serial-sim.py` 之類真子程序模擬器；把 trait mock 標成 simulator 是把證據等級高報一級 |
| L-37 | 原判 `implemented-and-connected` → 改判 `implemented-partial` | 「只在 refresh_tray 週期更新」對 embedded-runtime 模式**不成立**：`host_safety_relevant()` 含 `SensorStopUncertain`，`request_host_refresh()` 是合併式即時刷新且已接在事件轉送迴圈上、有單元測試。真正的缺口是**外部 daemon 模式沒有 `events.subscribe()`，只有輪詢**——是模式間不一致，不是全域 by-design，因此「無需修復」不成立 |
| L-41 | 原判 `implemented-partial` → 改判 `defined-only` | 該列 entryPath 自陳「無 production 呼叫端」與 `implemented-partial` 互斥；`migrate_legacy_pack` 連測試呼叫端都沒有（repo 內只有定義那一處），`accept_state`／`accept_state_with_epoch` 只有測試呼叫端，正式還原路徑 `CharacterSession::restore_report` 自己內聯實作 |
| L-43 | 同判 `implemented-and-connected` | 引用的測試檔 `settingsTransfer*.test.ts` **不存在**；真正的覆蓋在 `companion-gateway-wiring.test.ts`、`settings-transfer-validation.test.ts`、`settings-import-review.test.ts`、`deep-links.test.tsx`、`narrowNav.test.tsx`。兩個相容符號確實接在正式入口，狀態不變 |
| L-57 | 同判 `implemented-partial` | 「`pairingNotRecompared` 無下游消費者」不成立：`link_caps.rs:311-317` 把它寫進 `DriverReceipt`，`protocol_honesty.rs:1900` 有具名回歸。真正較窄的缺口是它沒被鏡射進 `declarative_session.rs` 的狀態快照（只有 `pairingUnverified` 有）。重啟後保守回落 `pairingUnverified` 那一半成立 |
| L-58 | 同判 `implemented-partial`（證據等級順序修正為 `unit`／`browser`） | browser 零覆蓋成立；被推翻的是「native 走查有（fixture）」——`scripts/tests/tauri-mobile-sensor-walkthrough.py:402` 自己記錄 `humanDismissal=False`、`deviceStopAcknowledgement=False`，那一步從沒跑過 |
| L-60 | 原判 `needs-investigation` → 改判 `confirmed-defect` | 靜態追蹤已能指到 `file:line`：重啟後 record 落 `Expired`＋detail「runtime restarted」，但 `workState.ts` 的 `expired` 投影是「已到期／muted」且**沒有 honesty 欄位**（只有 `unknown` 有「既不是成功也不是失敗」），`AiPage.tsx` 從不讀 `record.detail`，展開視圖只讀信箱。等於「結局未知」只活在 runtime 事件序列，產品層不揭露 |
| L-62 | 同判 `defined-only` | 結論（`timeline`／`realtime` 永遠協商不到）成立，理由錯：`SyncClass::{Timeline,Realtime}` 是 `crates/interaction-aip/src/capability.rs:23-27` 的真 enum，有 min-of-intersection 協商與具名單元測試；真正封死它的是 `crates/interaction-session/src/session.rs:469` 的 HostOffer 只放 `Semantic`。該列的小寫 grep 必然零命中，是假陰性 |
| L-64 | 同判 `implemented-and-connected` | 機制屬實（懷疑者實跑 `docs-claims.sh` 得到 188 passed / 0 failed；本文件撰稿時重跑同為 188 passed / 0 failed，完稿時為 212 passed / 0 failed——檢查條數會隨 repo 內容變動，見 §7）；被推翻的是 entryPath 的「CI Tauri backend job 亦跑」——`.github/workflows/` 對 `docs-claims`／`architecture-checks`／`drills` 零命中，唯一呼叫點是手動的 `scripts/release-verify.sh:100-105` |
| L-67 | 同判 `needs-investigation` | 三個子項之一被推翻：`docs/aip/adapter-development.md` §1 在 HEAD 已拆成 `### 1a`／`### 1b` 並明寫「這是**兩件事**…過去寫成同一步」。真正的問題方向相反——`docs/releases/v0.7.0-known-limitations.md:223`（§6.3）仍列為開放的「文件債」，而 `v0.8.0-known-limitations.md:24` 的「本輪已改動」清單沒有列 §6.3。另兩個子項（DESKTOP-GUIDE §4.7、v0.7.0 §3.2／§3.4／§4.5 未就地標註）成立 |
| L-68 | 原判 `implemented-partial` → 改判 `implemented-and-connected`（證據等級加 `integration`） | `http_loop.rs` 有兩支具名測試以真 axum server＋真 TCP loopback 走完整正式路徑（`a_timeout_after_the_request_was_sent_is_uncertain_and_never_resent`、`a_connect_failure_is_definitely_not_sent_so_it_may_be_retried`），把 `NotSent` 與 `OutcomeUnknown` 兩個分支都釘住了；這是完整而刻意的設計，不是半套 |
| L-69 | 原判 `needs-investigation` → 改判 `implemented-and-connected`（證據等級 `unit`／`fixture` → `unit`／`static-inspection`） | 反向承諾已由傳輸層結構性強制：`send_aip` 先 `ensure_ready_generation()`、`admit_aip` 先 `handshake_ready()`，且已有不同名的元件測試覆蓋兩側。缺的只是一支同名端到端回歸——那是覆蓋缺口，不是現況未明 |
| L-70 | 同判 `implemented-and-connected`（證據等級 `unit`／`native-desktop` → `unit`） | 引用的兩支 native 走查腳本完全沒碰 `_raw` 指令、hash 或 session 事件；真正的證據是 `apps/interaction-desktop/src/test/semantic-transport.test.ts` 這支 mock Tauri invoke 的單元測試 |
| L-71 | 原判 `implemented-partial` → 改判 `implemented-and-connected`（證據等級 `integration`／`static-inspection` → `static-inspection`） | `agent_request_allowed` 是 default-deny、對所有 HTTP method 生效，POST 白名單沒有 `/v1/memory` 也沒有 `/v1/memory/context-bundle`，因此是 **by construction 拒絕**而不是推論；缺的只是具名回歸測試（覆蓋缺口，不是實作缺口） |

## 4. 本輪新確認的產品缺陷 D1–D20

> 來源：`docs/releases/evidence/2026-09-07-phase-0/analysis/e2e-baseline.json` 的 `productDefects[]`。
> 這 20 條**不是靜態推論**：每一條都在本輪的真 agent、fixture 或原生走查裡實際發生過（或以直接讀碼確認為結構性事實，該條會註明）。
> 「本階段可修？」的判準只有一個：**它是否阻止本階段取得證據**。來源 JSON 的 `phase0MinimalFixCandidate`
> 在 D1–D4、D17–D19 都標 `true`，但依這個判準，**只有 D17／D18／D19 是 harness 缺陷（擋住驗證），D1–D4 是產品行為缺陷（沒有擋住驗證，本輪已經把它們驗出來了）**——本文件據此分開標，並保留來源欄位供整合者覆核。
> 證據路徑一律 repo 相對路徑；來源 JSON 裡指向 `…/home/…`（daemon token 與 SQLite）與 provider 私有目錄（`~/.claude`、`~/.codex`）的引用**依歸檔規則不入 repo**，本文件遇到會明說。

### 去重總表

| 缺陷 | 一句話 | 嚴重度 | 對應既有限制 | 建議階段 | 本階段可修？ |
|---|---|---|---|---|---|
| D1 | 人類中斷真 claude-code session 落 `failed` 而非 `cancelled` | high | **L-02（同根因合併）** | 1 | 否（產品缺陷，未阻止驗證）——**階段 1 已修（`24ae45d`）**，真 Claude 2.1.272 重跑終態 `cancelled` |
| D2 | close 的 SSE 投影把 `failed`／`timed-out`／`unknown` 壓成 `closed` | high | 與 L-60 同族（誠實階梯在投影層流失） | 1 | 否 |
| D3 | `failed` 的 session 在 record 與信箱上都沒有原因 | medium | 與 L-60 同族 | 1 | 否 |
| D4 | codex 核可「拒絕」送出的 wire 值 `reject` 不在 0.153.4 列舉內 | medium | 無（新） | 1 | 否——**階段 1 已修（`a24153e`）**，改送 `decline`（列舉由 D16 stderr 診斷紀錄取得） |
| D5 | codex 連接器沒有 MCP／plugin 封鎖，唯讀 session 仍啟動使用者的 MCP server | high | 與 L-07 同族（連接器能力不對稱） | 1 | 否 |
| D6 | codex `allowWrite` 不送 `writable_roots`，實際範圍併入使用者全域設定 | medium | 無（新） | 2 | 否 |
| D7 | codex 把所有帶 id+method 的 ServerRequest 都當核可請求 | medium | 與 L-01／D11 同族 | 2 | 否（EXPERIMENTAL，本輪未自然觸發） |
| D8 | 沒有「停止這一輪、保留 session」的能力 | medium | 與 L-59 同族（同 session 一次一輪） | 3 | 否 |
| D9 | resume guard 不論接受或拒絕都不留稽核 | medium | **L-04（補其未記的一面）** | 1 | 否 |
| D10 | SSE 混用 gateway 階段名與 record 狀態，同一刻兩個答案 | low | 無（新） | 2 | 否 |
| D11 | `waiting-for-input` 沒有任何連接器會自動產生 | medium | **L-01（同一件事）** | 2 | 否 |
| D12 | 受限 agent token 觸發 emergency stop 會讓 claimed 無法再被 verify | low | 無（新） | 3 | 否 |
| D13 | user-memory 沒有 supersede／conflict，矛盾偏好同時進 bundle | low | 與 L-11／L-12 同族（記憶邊界） | 4 | 否 |
| D14 | claimed-completed 的 session 可再收任務但 state 不反映該次執行 | low | 無（新，觀測性缺口） | 3 | 否 |
| D15 | daemon 只處理 SIGINT，SIGTERM 跳過 InstanceLock 的 Drop | low | 無（新） | 2 | 否 |
| D16 | codex 子程序 stderr 被完全丟棄，協定層錯誤不留痕 | low | 無（新） | 1 | 否 |
| D17 | fixture `fake_iphone` 連線失敗即 `process::exit(2)` | medium | 無（新，harness） | 1 | **是——已修，且已重跑驗證** |
| D18 | AX `choosefile` 無條件回報成功，不驗證是否真的導航 | medium | 與 L-48 同族（native 走查不在 CI） | 1 | **是——工作樹已修，未見重跑證據** |
| D19 | AX dump 遍歷整個視窗，隨 session 變慢並超過 55 s 上限 | medium | 與 L-48／L-58 同族 | 1 | **是——工作樹已修，未見重跑證據** |
| D20 | claude 續開 session 的 `providerSessionId` 在 create 回應仍是 null | low | 與 L-04 同族 | 2 | 否 |

### 本階段最小修復的當下狀態（撰稿時 `git status` 快照）

本階段只允許「阻止驗證的最小修復」，符合這個判準的只有三條 harness 缺陷。撰稿當下工作樹的狀態：

| 缺陷 | 修了嗎 | 修復後驗證了嗎 |
|---|---|---|
| D17 `fake_iphone` 硬退出 | **是**（`crates/interaction-runtime/examples/fake_iphone.rs`，未提交） | **是**——`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/run.txt` 的 F-06 四階段 PASS，fmt／clippy／build／`mobile_loop` 皆綠 |
| D18 AX `choosefile` 假陽性 | **是**（`scripts/lib/tauri-ax.applescript` 等四支腳本，未提交） | **否**——證據目錄下沒有修復後的 native 匯入重跑結果 |
| D19 AX dump 逾時 | **是**（timeout 55→150 s＋走訪改一次 `properties`，未提交） | **否**——證據目錄下沒有跑到 10/10 journey 的結果 |

> 這三條都**只動測試 harness，不動產品程式碼**。D18／D19 在補上重跑證據之前，E0-08／E0-11 的 `native-desktop` 欄位仍必須維持 `harness-failed`，不得因為「腳本已經改好」就改寫成通過。
> 另：本文件撰寫期間工作樹持續變動（其他階段 0 文件同時落盤、`README.md` 與 `docs/*.md` 被同批修改），本節是撰稿當下的快照，整合者請以自己的 `git status` 為準。
>
> **整合者更新（2026-09-14）**：上表「未提交」的三條修復已提交到分支 `phase-0/state-recovery-baseline`——D17 為 `88181c0`，D18／D19 為 `9384fd5`（[進度入口 §8](phase-0-progress.md)）。D18／D19「修復後驗證了嗎」仍是**否**：沒有重跑 native 走查，E0-03／E0-08／E0-11 的 native-desktop 維持 harness-failed。

### D1 — 人類 interrupt 真 claude-code session → 終態 `failed` 而非 `cancelled`；Claude 連接器在任何路徑都產不出 `cancelled`

- **嚴重度**：high｜**建議階段**：1｜**對應既有限制**：L-02（同根因合併）
- **位置**：`crates/interaction-agent-gateway/src/claude.rs:354-357`（`interrupt` 只送 SIGINT，不留取消旗標）＋`claude.rs:442-455`（`result/subtype` 以 error 開頭 → `TaskFailed`）＋`claude.rs:213-231`（`TaskFailed` 設 `saw_result`，讓 `:267-283` 的誠實退路失效）→ `crates/interaction-runtime/src/gateway.rs:579-584` → `crates/interaction-runtime/src/agents.rs:1185`
- **重現**：`python3 scripts/tests/phase0/agent_smoke.py --agent claude-code --port <N> --out <DIR> --task '<約 1500 字長文>' --cancel-after 3 --cancel-when-active --cancel-mode interrupt` → 終態 `failed`（期望 `cancelled`）；同旗標換 `--agent codex` → `cancelled`
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1/claude-interrupt/result.json`（cancel-issued 09:05:00.474 → state failed 09:05:01.002）、同目錄 `sse.jsonl`（第 109 行 `state=failed`、第 102 行 character system-text intent=failed「動作失敗。」）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1/b07-claude/result.json`（獨立第二次）、`docs/releases/evidence/2026-09-07-phase-0/real-agent-e2e-prior/{claude-cancel-1,multi-C-claude-x2,multi-E-claude-codex}/result.json`（另外三次，合計 5/5）。對照組正確行為：`crates/interaction-agent-gateway/src/codex_exec.rs:253-258,320-329`（先設 `cancel_requested` 再送訊號）
- **撰稿時獨立複核**：`grep -c TaskCancelled crates/interaction-agent-gateway/src/claude.rs` → `0`；同一 grep 對 `codex.rs` → `2`、`codex_exec.rs` → `4`
- **未歸檔的來源引用**：`e2e/claude-cancel-1/home/state/interaction.db`（observation 內 `error_during_execution`）與 `~/.claude/projects/…jsonl:9`（`[Request interrupted by user]`）依歸檔規則不入 repo

### D2 — close 的 SSE 投影把已終結的 `failed`／`timed-out`／`unknown` 一律壓成 `closed`

- **嚴重度**：high｜**建議階段**：1｜**對應既有限制**：與 L-60 同族
- **位置**：`crates/interaction-runtime/src/agents.rs:1371-1379`（只特判 `Cancelled`，其餘落 `"closed"`），對照 `agents.rs:1296-1310`（record 刻意保留前一個終局狀態，並有反回歸註解）
- **重現**：中斷一個 claude session（state→failed），再 `POST /v1/agent-sessions/{id}/close`；close 回應與之後的 GET 都是 `failed`，但同一瞬間發出的 SSE `agent.session.state` 是 `closed`
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1/claude-interrupt/sse.jsonl`（第 116 行 `state='closed'` @2026-09-07T09:05:07.606889Z）對照同目錄 `result.json`（`final.state='failed'`、`closedAt=09:05:07.598639Z`）；`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1/codex-interrupt/sse.jsonl`（第 109 行 `cancelled` — `Cancelled` 是唯一被保留的終局狀態）
- **撰稿時獨立複核**：`agents.rs:1371-1379` 的 `if record.state == AgentSessionState::Cancelled { "cancelled" } else { "closed" }` 逐字相符
- **階段 1 處置**：**已修**（`d8f934b`，`fix(runtime): persist the execution phase and keep terminal outcomes on close (D10)`；詳見 `docs/aip/interaction-tracing.md` §4）

### D3 — `failed` 的 agent session 在 record 與 mailbox 上都沒有任何原因

- **嚴重度**：medium｜**建議階段**：1｜**對應既有限制**：與 L-60 同族
- **位置**：`crates/interaction-runtime/src/agents.rs:1209-1218`（`report_agent_session` 只設 state，從不寫 `entry.record.detail`）；錯誤 payload 只進 `ingest("agent.session", …)`（`agents.rs:1254-1257`）
- **重現**：中斷一個進行中的 claude session → `GET /v1/agent-sessions/{id}` 得 `{"state":"failed"}` 且無 `detail`；`GET …/messages?direction=from-session` → `[]`；只有 observations 查得到 `error_during_execution`
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1/b07-claude/result.json`（完整 record，`state='failed'`，無 detail key）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1/diag-failreason/result.json`（`detail=null`、from-session 信箱 0 筆、audit 無對應行、observations 最新 `inferences.report.error='error_during_execution'`）
- **撰稿時獨立複核**：`agents.rs:1209-1218` 確實只做 `entry.record.state = next_state`（＋claim id／清 human_verified），沒有寫 detail
- **階段 1 處置**：**已修**（`ce68181`，`feat(runtime): dispatch, delivery, outcome and stderr diagnostic records`；連接器錯誤前 200 字同時寫入 `AgentSessionRecord.detail` 與 audit `agent-session.outcome` 的 `detail.reason`，詳見 `docs/aip/interaction-tracing.md` §4）

### D4 — Codex 核可「拒絕」送出的 wire 值 `reject` 不在 codex 0.153.4 的列舉內

- **嚴重度**：medium｜**建議階段**：1｜**對應既有限制**：無（新）
- **位置**：`crates/interaction-agent-gateway/src/codex.rs:643-644`（`Approve => "accept"`、`Deny => "reject"`）
- **重現**：`unset INTERACT_AI_CLAUDE_BIN INTERACT_AI_CODEX_BIN; python3 scripts/tests/phase0/agent_smoke.py --agent codex --port <N> --out <DIR> --approve deny --task '請讀取 NOTES.md 並回答第一個標題。' --timeout 90`，再讀 `~/.codex/sessions/<Y>/<M>/<D>/rollout-*-<providerSessionId>.jsonl` 的 `custom_tool_call_output`
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R4/a-codex-deny/result.json`（agent 把 `approval request failed` 原文轉述給人類）；對照組 `docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R4/c-codex-write/`（`accept` 是合法值 → 指令真的執行）
- **撰稿時獨立複核**（第一手）：`strings -a /opt/homebrew/Caskroom/codex/0.153.4/bin/codex | grep -o 'acceptForSession[a-zA-Z]*'` → `acceptForSessiondeclinecancel…`，二進位內**沒有** `reject` 變體；`codex.rs:643-644` 逐字相符
- **未歸檔的來源引用**：`matrix/codex-schema-0.153.4/*.json`（schema dump）產於前一輪 scratchpad，repo 內查不到；`~/.codex/sessions/…` 為 provider 私有目錄，不歸檔

### D5 — codex 連接器沒有等價於 claude 的 MCP／plugin 封鎖

- **嚴重度**：high｜**建議階段**：1｜**對應既有限制**：與 L-07 同族（連接器能力不對稱）
- **位置**：`crates/interaction-agent-gateway/src/codex.rs:166-169`（只設 `current_dir`，無任何 MCP／plugin 限制），對照 `claude.rs:82-84`（`--strict-mcp-config --mcp-config {"mcpServers":{}}`）
- **影響**：宣告 `allowWrite=false`、`toolScope=[]` 的唯讀 session 一建立就會啟動使用者 `~/.codex` 設定裡的 MCP server（本輪實測命中瀏覽器控制與程式碼索引兩支）與 ChatGPT computer-use helper；任務全文出現在 world-readable 的 process argv
- **重現**：見 `docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R8/`：建立唯讀 codex session、送含唯一標記的任務，同時每 80 ms 掃 `ps -axww -o pid=,ppid=,command=`
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R8/argv-codex/run.txt`（第 9–11 行：`npx -y chrome-devtools-mcp@latest`、`codegraph serve --mcp`、ChatGPT `unified-computer-use` 三個子程序，ppid 為 codex app-server）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R8/psleak/raw.json`（`$.psHits[0].line` 含唯一標記與完整 input-messages）、對照組 `docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R8/argv-claude/run.txt`（claude argv 有 `--strict-mcp-config`）
- **撰稿時獨立複核**：`codex.rs:164-170` 只有 `remove_runtime_auth_env`／`apply_session_capability_env`／`current_dir`／`arg("app-server")`；`claude.rs:80-86` 確有 `--strict-mcp-config` 與空 `mcpServers`

### D6 — `allowWrite` 的 codex session 只送 `sandbox:"workspace-write"` 字串、不送 `writable_roots`

- **嚴重度**：medium｜**建議階段**：2｜**對應既有限制**：無（新）
- **位置**：`crates/interaction-agent-gateway/src/codex.rs:315-340`（`thread/start` 與 `thread/resume` 的 params 只有 `cwd`／`approvalPolicy`／`sandbox` 字串）
- **影響**：實際生效的可寫範圍併入使用者全域 `~/.codex/config.toml` 的 `[sandbox_workspace_write] writable_roots`，超出 session 授權的 `resolvedWorkdir`
- **重現**：在 `~/.codex/config.toml` 設 `writable_roots`；用 `--allow-write` 建 codex session 送任務；讀 rollout 的 `turn_context.payload.sandbox_policy`
- **證據**：來源引用的 rollout 與使用者 config 皆在 provider 私有目錄，**依歸檔規則不入 repo**；repo 內可核的是 `codex.rs:315-340` 的 params 形狀（靜態）

### D7 — codex.rs 把所有帶 id+method 的 ServerRequest 一律當核可請求

- **嚴重度**：medium｜**建議階段**：2｜**對應既有限制**：與 L-01／D11 同族
- **位置**：`crates/interaction-agent-gateway/src/codex.rs:250-263`（`(true, Some(m))` 分支不 dispatch method）＋`codex.rs:432-445`（`approval_summary` 只找 command/cmd/path/reason）＋`codex.rs:623-660`（`resolve_approval` 回 `{result:{decision}}`）
- **狀態限定**：**EXPERIMENTAL**——本輪兩次真 turn 都沒有自然觸發 `item/tool/requestUserInput`，因此這條是**直接讀碼＋schema 靜態比對**的結論，不是實跑重現
- **重現**：讓 codex app-server 對某 thread 送出 `method=item/tool/requestUserInput` 的 ServerRequest（本輪未觸發）；或靜態比對 `codex.rs:646-650` 送出的 JSON 與 `ToolRequestUserInputResponse` 的必填欄位
- **證據**：`codex.rs:249-266,431-445,623-660`（讀碼）；schema dump 產於前一輪 scratchpad，**repo 內查不到**

### D8 — 沒有「停止這一輪、保留 session」的能力

- **嚴重度**：medium｜**建議階段**：3｜**對應既有限制**：與 L-59 同族
- **位置**：`crates/interaction-runtime/src/agents.rs:900-904`（state 非 open 即拒絕）＋`crates/interaction-runtime/src/gateway.rs:579-588`（interrupt 轉成終局回報而非只結束 turn）
- **重現**：長任務 → 等 active → `POST /interrupt` → 等終態 → 再 `POST …/messages {kind:task}`
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1/b07-claude/result.json`（`task2Status=409 'agent session … is Failed; mailbox closed'`）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1/b07-codex/result.json`（`… is Cancelled; mailbox closed`）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1/b07-claude/result.json` 的 `events[k=children-final].children=[]`（子程序樹已消失）
- **註**：provider 端其實只中止 turn（codex rollout 記 `turn_aborted reason=interrupted`，該檔在 provider 私有目錄不歸檔）；是 runtime 選擇殺掉整個 session

### D9 — 續開授權檢查不論拒絕或接受都不留稽核

- **嚴重度**：medium｜**建議階段**：1｜**對應既有限制**：L-04（補其未記的一面）
- **位置**：`crates/interaction-runtime/src/agents.rs:453-468`（resume guard 直接 `return Err(PolicyBlocked)`，整段沒有 `store.audit`）；`check_resume_not_wider`（`:105`）、`check_resume_same_workdir`（`:204`）同樣無 audit
- **重現**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R8/` 的 `guard_probe.py`：對 7 個「放寬」形狀各建一次 session（皆 403），之後 `GET /v1/audit` 找不到任何對應紀錄
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R8/guard-claude/raw.json`（`$.auditAfter` 47 筆，kind 只有 character／agent-session.closed／capability-issued）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R8/claude/raw.json`（`$.auditTail` 23 筆，同樣沒有 resume／policy_blocked）
- **階段 1 處置**：**已修**（`5eff899`，`fix(runtime): audit every resume authorization decision (D9)`；詳見 `docs/aip/interaction-tracing.md` §3）

### D10 — SSE 的 `agent.session.state` 混用 gateway 階段名與真實 record 狀態

- **嚴重度**：low｜**建議階段**：2｜**對應既有限制**：無（新）
- **位置**：`crates/interaction-runtime/src/gateway.rs:798`（emit `"fetched"`）與 `crates/interaction-runtime/src/agents.rs:1060`（`"working"`）；`AgentSessionRecord` 沒有對應狀態
- **重現**：建立 claude session 送任務，同時開 SSE 與每 0.5 s `GET /v1/agent-sessions/{id}`：整段等待期間 GET 回 `created` 而 SSE 已是 `fetched`
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1/claude-interrupt/result.json`（task-sent 到 active 61.3 s，期間 GET 一律 `created`）、`docs/releases/evidence/2026-09-07-phase-0/real-agent-e2e-prior/claude-cancel-1/sse.jsonl`（`id=99 state='fetched'`）
- **註**：桌面把兩個值都投影成「準備中」，所以不造成錯誤呈現；問題是「取消當下是什麼狀態」有兩個答案
- **引用更正**：來源 JSON 把 `"working"` 標在 `agents.rs:1060`，撰稿時實讀該行是 `emit_agent_session_state(… "fetched")`；`"working"` 實際在 `crates/interaction-runtime/src/agents.rs:1247`（`"task-started" | "progress" => "working"`）。結論不變、行號要改
- **階段 1 處置**：**已修**（`d8f934b`，`fix(runtime): persist the execution phase and keep terminal outcomes on close (D10)`；三維拆分 `phase`／`recordState`／`lifecycle`，詳見 `docs/aip/interaction-tracing.md` §4）

### D11 — `waiting-for-input` 沒有任何連接器會自動產生

- **嚴重度**：medium｜**建議階段**：2｜**對應既有限制**：**L-01（同一件事，合併）**
- **位置**：`crates/interaction-agent-gateway/src/lib.rs:99`（定義＋自陳註解）；`crates/interaction-runtime/src/gateway.rs:412-417`（唯一消費點）；`crates/interaction-runtime/src/agents.rs:1182,1248`（只走人工回報路徑）
- **重現**：`grep -rn TaskWaitingForInput crates/interaction-agent-gateway/src/`（只命中定義）；跑一次真 agent 的澄清問答 → 澄清問句被包成 `claimed-completed`，從未出現 `waiting-for-input`
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R3/claude-followup/result.json`（turn1 終態 `claimed-completed`，summary 是澄清問句）；UI 側投影已存在於 `apps/interaction-desktop/src/statusProjection/workState.ts:91-101`
- **撰稿時獨立複核**：`grep -rn TaskWaitingForInput crates/interaction-agent-gateway/src/` → 只有 `lib.rs:99`

### D12 — 受限 agent token 觸發 emergency stop 會讓 claimed 的 session 再也無法 verify

- **嚴重度**：low｜**建議階段**：3｜**對應既有限制**：無（新）
- **位置**：`crates/interaction-api/src/lib.rs:454-461`（agent token 允許 `POST /v1/emergency-stop`）＋`crates/interaction-runtime/src/agents.rs:1120-1150`（只有 claimed-completed 可 verify）
- **重現**：建一個 claude session 跑到 `claimed-completed`（先不 verify）→ 用 `<home>/state/api-agent-token` 呼叫 `POST /v1/emergency-stop` → session 變 `cancelled` → `POST …/verify` 回 409
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R4/f-agent-token/steps.json`（`agent-estop-set` @2026-09-07T09:09:59Z，status 200，actor=agent）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R4/c-claude-write/restart-steps.json`（同一秒 `state=cancelled`）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R4/c-claude-write/verify-after-restart.json`（409 `only a claimed-completed session can be verified`）
- **註**：這**不違反**「AI 不可解除 emergency stop」——AI 只能觸發、不能解除；問題是觸發的副作用會抹掉人類尚未做的驗證機會

### D13 — user-memory／preference 層沒有 supersede／conflict

- **嚴重度**：low｜**建議階段**：4｜**對應既有限制**：與 L-11／L-12 同族
- **位置**：`crates/interaction-runtime/src/memory.rs:279-353`（bundle 建構無 supersede／dedup），對照 `crates/interaction-runtime/src/knowledge.rs:1182-1225`（知識層已有 candidate→active→stale→disputed→superseded 狀態機）
- **重現**：建立兩筆同 layer、同 title/tag、內容矛盾的 preference，呼叫 `POST /v1/memory/context-bundle`：`includes` 同時含兩筆，彼此無 `supersededBy`／`conflictWith`
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R2/step4-bundle-after-mem2.json`（includes 兩筆矛盾偏好）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R2/E0-05-supersede/result.json`（agent 只能靠文字推論新舊）

### D14 — claimed-completed 的 session 仍可收新任務並執行，但 state 全程不反映

- **嚴重度**：low｜**建議階段**：3｜**對應既有限制**：無（新，觀測性缺口而非行為缺陷）
- **位置**：`crates/interaction-core/src/agent.rs:48-55`（`is_open()` 含 `ClaimedCompleted`，並有回歸測試註明「驗證與關閉是人類／runtime 的獨立步驟」）；`crates/interaction-runtime/src/agents.rs:899-905`
- **重現**：讓 claude session 走到 `claimed-completed`，對同一 sessionId 再 `POST {kind:task,…}`，重複 GET：state 全程停在 `claimed-completed`
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R2/step5-old-session-resend.json`（POST 200，`contextBundle.includes=[]`）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R2/step5-old-session-final.json`（唯一擷取到的 GET）
- **註**：來源 JSON 明記——原報告聲稱的另一組 spent 數字**沒有任何擷取到的回應支持，已由懷疑者刪除**

### D15 — daemon 只處理 SIGINT 的優雅關閉

- **嚴重度**：low｜**建議階段**：2｜**對應既有限制**：無（新）
- **位置**：`crates/interaction-cli/src/commands.rs:1608`（只 `await tokio::signal::ctrl_c()`）；`crates/interaction-runtime/src/lock.rs:84-94`（`impl Drop for InstanceLock`）
- **重現**：啟動 `interact-ai serve`，`kill -TERM <pid>`；程序立即消失且不印 shutting down；以相同 home 再次啟動會印 `WARN … reclaiming stale instance lock pid=<old>`
- **證據**：來源引用的 `runs/R5/home{A,B}/daemon-run2.log` 位於名稱含 `home` 的目錄，**依歸檔規則未入 repo**；repo 內可核的是上述兩處 `file:line`（靜態）。要重新取得執行期證據請用上面的重現步驟

### D16 — codex 子程序 stderr 被完全丟棄

- **嚴重度**：low｜**建議階段**：1｜**對應既有限制**：無（新）
- **位置**：`crates/interaction-agent-gateway/src/codex.rs:176-181`（`while let Ok(Some(_)) = lines.next_line().await {}`，註解自陳「診斷輸出，吞掉避免管線塞住」）
- **重現**：跑 D4 的 deny 案例後 `grep -iE 'error|reject|approval|decision' <out>/daemon.txt` → 0 筆
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R4/a-codex-deny/daemon.txt`（全檔只有啟動訊息）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R4/b-codex/daemon.txt`（同上）
- **撰稿時獨立複核**：`codex.rs:176-181` 逐字相符
- **階段 1 處置**：**已修**（`baabec7`，`fix(gateway): keep a bounded, redacted stderr tail for every agent subprocess (D16)`；三個連接器改用共用的 `crates/interaction-agent-gateway/src/diagnostics.rs`，詳見 `docs/aip/interaction-tracing.md` §5）

### D17 — fixture `fake_iphone` 連線失敗即 `process::exit(2)`（harness 缺陷）

- **嚴重度**：medium｜**建議階段**：1｜**對應既有限制**：無（新）
- **位置（修復前）**：`crates/interaction-runtime/examples/fake_iphone.rs` 的 `die()`／`connect()`／`reconnect()`——`reconnect` 無條件呼叫 `connect`，`connect` 失敗即 `process::exit(2)`
- **影響**：E0-08 的 F-06「核心離線→重啟→同一個 process 用同 token 重連」在出貨 fixture 上走不完，本輪只能改用自寫 WS probe 驗 daemon 側
- **證據（缺陷）**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R6/logs/phone2.stderr.txt`、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R6/logs/phone2.txt`（最後一行後程序從 ps 消失）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R6/logs/probe-{while-dead,after-restart}.json`（改用自寫 probe 才驗到 daemon 側行為正確）
- **本階段處置**：**已修**。工作樹已有未提交修改（`git diff crates/interaction-runtime/examples/fake_iphone.rs`）：`connect()` 改回傳 `Result`，`pair()` 連不上仍視為致命，`reconnect()` 改發 `{"event":"reconnect-failed",…}` 並保持程序存活
- **修復後的驗證證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/run.txt`
  ——PHASE B「SIGKILL daemon → reconnect → `reconnect-failed` 且 process 存活」PASS、PHASE C「同 home 重啟 → 同一 process 重連成功」PASS、PHASE D「撤銷後 reconnect → `auth-fail` 且 process 存活」PASS；
  同一 run 目錄下的 `docs/releases/evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/cargo-fmt.txt`、`…/logs/cargo-clippy.txt`、`…/logs/cargo-build-example.txt` 皆 exit 0，`…/logs/cargo-test-mobile-loop.txt` 顯示 `81 passed; 0 failed`
- **注意**：此修復**只動測試 fixture，不動產品程式碼**；重現／驗證等級是 `fixture`，不是 real-iphone

### D18 — AX 測試輔助的 `choosefile` 無條件回報成功（harness 缺陷）

- **嚴重度**：medium｜**建議階段**：1｜**對應既有限制**：與 L-48 同族
- **位置**：`scripts/lib/tauri-ax.applescript:51-63`（Cmd-Shift-G → 固定 delay → 輸入路徑 → Return → 固定 delay → Return → **無條件** return「selected file in owned Open panel」）
- **影響**：Open panel 會停在記憶中的舊目錄並開字母序第一個檔，於是「匯入了錯誤的檔」也會被記成成功——本輪 E0-11 的 native 匯入與四個阻擋案例全部無法驗證，且 **2026-09-06 那次的綠燈有假陽性之虞**
- **重現**：以刻意不合法的 `kind` 觸發匯入，UI 仍顯示「已匯入角色設定並套用。」（解析器本應拒絕）
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R7/probe5/main-window-final.txt`（第 225 行「已匯入角色設定並套用。」）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R7/probe1/prefs-final.json`、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R7/settings/result.json`；解析器應拒絕的依據在 `apps/interaction-desktop/src/companion/settingsTransfer.ts:110-132`
- **本階段處置**：**工作樹已有未提交修復，但本文件找不到修復後的重跑證據**。
  撰稿當下 `git status` 顯示 `scripts/lib/tauri-ax.applescript`、`scripts/tests/tauri-settings-walkthrough.py`、`scripts/tests/tauri-work-cancel.py` 皆為已修改未提交；
  修改後的 `choosefile` 改成「Go-to-Folder 貼上路徑 → 讀回比對 → 斷言面板真的到了該目錄（Where: 欄）→ 才開檔」，任何一步不成立就 `error`
  （檔頭第 12–17 行明寫「沒有讀回就回報成功等於製造假陽性（2026-09-06／09-07 的 choosefile 就是這樣）」；實作在同檔 `:157-262`）。
  **對照 HEAD**：`git show HEAD:scripts/lib/tauri-ax.applescript | sed -n '51,63p'` 仍是「固定 delay → 兩次 Return → 無條件回報成功」。
  **尚未成立的部分**：`docs/releases/evidence/2026-09-07-phase-0/` 內**沒有**修復後重跑 native 匯入走查的結果檔；
  在補上那份證據之前，這條只能記成「已修、未驗證」，而 2026-09-06 的綠燈是否為假陽性也仍未重新裁決

### D19 — AX dump 遍歷整個視窗，隨 session 累積而變慢並超過 55 s 上限（harness 缺陷）

- **嚴重度**：medium｜**建議階段**：1｜**對應既有限制**：與 L-48／L-58 同族
- **位置**：`scripts/lib/tauri-ax.applescript` 的 dump handler（走 System Events 的 entire contents）＋`scripts/tests/tauri-mobile-sensor-walkthrough.py:225`（subprocess timeout 55 s）
- **影響**：E0-08 native 的四個 sensor journey 從未跑到；E0-03 native 也連兩次卡在 AX 輸入
- **重現**：`python3 scripts/tests/tauri-mobile-sensor-walkthrough.py --app <App> --out <dir>`，觀察 `$.ax` 的 dump 耗時單調上升直到逾時
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R7/mobile/result.json`（4/10 journey，312.414 s 後 dump 逾時）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R7/mobile-diag1/result.json`（6/10 journey，430.024 s）；對照：同一顆 App binary 於 2026-09-06 曾完成 10/10（`docs/releases/evidence/2026-09-06-convergence/final/native-final-checkpoint/mobile-clean/result.json`）
- **本輪的定位嘗試**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/H-native-attempt1-interrupted/probe/p1/trace.json`
  ——`slowdump` 10.166 s 成功、`batchdump` exit 1 且 `dump-fast.txt` 為空，**該輪被中斷，未得出結論**
- **本階段處置**：**工作樹已有未提交修復，但本文件找不到修復後的重跑證據**。
  撰稿當下 `scripts/tests/tauri-mobile-sensor-walkthrough.py` 的 subprocess timeout 已由 55 s 改為 150 s，
  且 `scripts/lib/tauri-ax.applescript` 的走訪改成「每個元素只抓一次 `properties`」（檔頭第 7–11 行自陳原因就是 2026-09-07 的 mobile 走查掛在 dump 上）。
  **對照 HEAD**：`git show HEAD:scripts/tests/tauri-mobile-sensor-walkthrough.py | sed -n '226p'` 仍是 `timeout=55`。
  **尚未成立的部分**：`docs/releases/evidence/2026-09-07-phase-0/` 內**沒有**修復後跑到 10/10 journey 的結果檔；
  在補上那份證據之前，E0-08 native 仍是 `harness-failed`，不得改寫成已通過

### D20 — claude 續開 session 的 `providerSessionId` 在 create 回應仍是 null

- **嚴重度**：low｜**建議階段**：2｜**對應既有限制**：與 L-04 同族
- **位置**：`crates/interaction-runtime/src/agents.rs:547`（`provider_session_id: None` 建檔）與 `:619-621`（等 connector init 事件回填）；`CreateAgentSession.resume_provider_session_id`（`agents.rs:277`）未寫進 `AgentSessionRecord`
- **重現**：以 `--resume-id <psid>` 建立三個被接受的續開 session，看 create 回應的 `providerSessionId`
- **證據**：`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R8/guard-claude/raw.json`（`$.probes[?(@.name=='ttl-narrower')].record.providerSessionId` → null）、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R8/claude/raw.json`（`$.s2.createRecord.providerSessionId` → null，但 `$.cases.S2T1.providerSessionId` 有值）、對照 `docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R8/codex/raw.json`（codex 在 attach 當下就有值）

## 5. 環境阻礙與需要的人類操作

> 來源：`docs/releases/evidence/2026-09-07-phase-0/analysis/e2e-baseline.json` 的 `environmentBlockers[]`。
> 這些**不是產品缺陷**，是本輪拿不到某一級證據的原因。每一條都寫明「誰要做什麼」——本階段沒有代人做，也沒有繞過。

| 代號 | 阻礙 | 擋住哪些證據 | 需要的人類操作 |
|---|---|---|---|
| B1 | **真 iPhone 簽章**：iPhone 11「Alex」已連線（`available (paired)`）且 Developer Mode 已開，但 Xcode 的 `IDEProvisioningTeams` 為空（keychain 內雖有 Apple Development 憑證，那不是 `xcodebuild -allowProvisioningUpdates` 讀的 store），`apps/interaction-ios/scripts/device-build.sh --check-only` 卡在 step 3/5 | E0-08 的 `real-iphone`；**任何真機驗收欄位**（本輪 real-iphone = 0 筆） | 在 Xcode → Settings → Accounts 登入 Apple ID 並選 Team，之後重跑 `device-build.sh --check-only` 通過再做真機 E2E |
| B2 | **macOS 合成鍵盤事件遺失**：AppleScript `keystroke` 打進 Tauri WebView 會非決定性掉字（兩次執行分別得到不同的截斷結果，`osascript` 全部 exit 0）。發生在 System Events 投遞或 WebView／React 受控輸入未定；本輪不得改 repo 檔案故未進一步定位 | E0-03 `native-desktop`、E0-02 的 native 派工 | 在低負載機器上重跑以區分負載因素；或授權修改 `scripts/tests/tauri-work-cancel.py` 改用剪貼簿貼上／逐字元驗證後再跑 |
| B3 | **NSOpenPanel 記憶目錄＋未驗證的 `choosefile`**：Open panel 停在 2026-09-06 的目錄，AX helper 無條件回報成功（＝ D18） | E0-11 `native-desktop` 的匯入半邊；並使 2026-09-06 的綠燈有假陽性之虞 | 授權修改 `scripts/lib/tauri-ax.applescript` 加上「確認選到的檔名」斷言，並重新驗證 2026-09-06 的 `settings-clean`／`settings-legacy` 綠燈 |
| B4 | **磁碟空間**：本輪禁止 `cargo build`／`pnpm build`，原生 App 無法從 HEAD 重建；所有 `native-desktop` 結果綁在 2026-09-06 的 bundle（`Info.plist` 0.7.0，`appSourceRef` 不是 HEAD 的祖先） | E0-01／E0-03／E0-08／E0-11 的 `native-desktop` 等級 | 清出磁碟空間後從 HEAD 重建 `apps/interaction-desktop`，再重跑 native 基線 |
| B5 | **Claude 額度／冷啟動**：R1 的五次 claude 執行從 task-sent 到 active 一律約 61–65 s；上一輪另有一次送出任務後 21 s 仍是 `created` 且 provider log 沒有任何 assistant 紀錄（時間接近額度重置窗口，合理但**未證實**） | E0-02／E0-03／E0-04（未進 active 的那一次），以及任何有時間斷言的 case | 避開額度重置窗口重跑一次同型測試並開 debug 級 daemon 日誌，或改用不受額度影響的帳號，才能判斷 61 s 冷啟動是不是產品層問題 |
| B2′／B3′ | **上列 B2、B3 的當下狀態**：撰稿時 `git status` 顯示 `scripts/lib/tauri-ax.applescript`、`scripts/tests/tauri-work-cancel.py`、`scripts/tests/tauri-settings-walkthrough.py`、`scripts/tests/tauri-mobile-sensor-walkthrough.py` 皆已修改未提交——合成鍵盤輸入改為剪貼簿貼上＋讀回比對，`choosefile` 改為讀回＋導航斷言（見 D18／D19） | 同 B2／B3 | **仍缺**：修復後的 native 重跑結果尚未出現在 `docs/releases/evidence/2026-09-07-phase-0/`；B4（磁碟）也還沒解，App 仍是 2026-09-06 的 bundle |
| B6 | **共用機器＋多個並行 agent**：整輪期間有其他 agent 的 `interact-ai` daemon／`claude -p`／codex app-server 同時在跑（各回合懷疑者分別點名多個 PID，皆屬其他 out 目錄），所有 `ps` 斷言都必須依 ppid／pgid 鏈過濾 | 所有以「子程序存活與否」為證據的 case | 由人類決定這些 sibling run 的殘留 daemon 由誰回收；**本輪任何 agent 都不得 `pkill -f interact-ai`** |

**B4 的當下數字（撰稿時實測，供整合者判斷可否重建）**：`df -h /` 顯示可用 9.3 GiB，`du -sh target` 為 3.5 GB。
與 `docs/releases/phase-0-repository-state.md` 記錄的「續接時 12 GiB 可用、`target/` 7.4 GB」不同，代表磁碟狀況會漂移，重建前要重新量。

## 6. 已知限制清單與程式現況的不一致（供整合者更新 `CHANGELOG.md`／known-limitations）

> 這一節只放**可以據以改文件**的項目。每一條都標「方向」與「可核對的依據」。
> 本文件**沒有**改任何既有清單（不在本文件的可寫範圍內）。

### 6.1 文件說有、程式已修掉（過期限制，應就地標註或劃掉）

| # | 文件位置 | 文件的說法 | 現況與依據 |
|---|---|---|---|
| 1 | `docs/releases/v0.7.0-known-limitations.md:183`（§3.2） | rebind 後動器 `enabled` 旗標回預設「**沒有測試釘住**」 | 已有具名測試 `rebind_never_restores_external_actuator_enabled_true`（撰稿時 grep 命中 `crates/interaction-runtime/tests/declarative_session_loop.rs:2462`）。對應 L-26；`v0.8.0-known-limitations.md:24` 已概括，但 v0.7.0 檔未就地標註 |
| 2 | `docs/releases/v0.7.0-known-limitations.md:185`（§3.4） | `unresolvedStops` 只在記憶體、不跨重啟（狀態欄「未做」） | v0.8.0 已以 SQLite sensor journal 持久化（`crates/interaction-runtime/src/sensor_journal.rs` 存在）。對應 L-21；同樣未就地標註 |
| 3 | `docs/releases/v0.7.0-known-limitations.md:200`（§4.5） | preset `opId` 由 presetId＋ms 決定會同名 | 已改用 `crypto.randomUUID()`（撰稿時 grep：`apps/interaction-desktop/src/companion/applyPresetPlan.ts:73`）。對應 L-36；同樣未就地標註 |
| 4 | `docs/releases/v0.7.0-known-limitations.md:223`（§6.3） | `docs/aip/adapter-development.md` §1 把「加一個角色」與「加一個新 adapter」混成一步（狀態欄「文件債」） | HEAD 已拆成 `### 1a. 加一個角色`／`### 1b. 加一個新 adapter`，並在 §1 開頭明寫「這是**兩件事**…它們過去寫成同一步」（撰稿時實讀 `docs/aip/adapter-development.md:6-30`）。**且 `v0.8.0-known-limitations.md:24` 的「本輪已改動」清單沒有列 §6.3** ——這一條是懷疑者對 L-67 的改判結果 |
| 5 | `docs/acceptance-evidence.md:1005` | 「`release.yml` 的 `ci-gate`／`finalize` 兩個 job **未經真實 tag push 驗證**（證據等級：unit）」 | v0.8.0 已真的 tag push 並跑完 Release workflow：`docs/releases/v0.8.0-publication.md:17`（`release-tag.sh 0.8.0 --push`，exit 0）與 `:18`（Release workflow run，11 個 job 全 success）；`.github/workflows/release.yml` 的 `ci-gate`（:34）與 `finalize`（:252）確實是該 workflow 的 job。**這一條由 completeness critic 提出（附錄 A #15），本文件已獨立核對上述四處，因此列在正式清單而非附錄** |

### 6.2 文件（或矩陣）說沒有／未驗，程式其實已有

| # | 說法出處 | 說法 | 現況與依據 |
|---|---|---|---|
| 1 | v0.5.1 沿承（L-57） | `pairingNotRecompared` **無下游消費者** | `crates/interaction-adapter-declarative/src/link_caps.rs:317` 把它寫進 `DriverReceipt`，`crates/interaction-adapter-declarative/tests/protocol_honesty.rs:1900` 有具名回歸（撰稿時 grep 命中）。真正較窄的缺口是它沒被鏡射進 `declarative_session.rs` 的狀態快照 |
| 2 | v0.5.1 保留 #12（L-68） | HTTP 逾時分類「測試覆蓋未核對」 | `crates/interaction-adapter-declarative/tests/http_loop.rs:265`／`:334` 兩支具名測試以真 axum server＋真 TCP loopback 覆蓋 `OutcomeUnknown` 與 `NotSent` 兩個分支（撰稿時 grep 命中） |
| 3 | 矩陣 L-62 自述 | `timeline`／`realtime` 值「只在 schema JSON 與文件」 | `crates/interaction-aip/src/capability.rs:23-27` 有 `SyncClass::{Semantic,Timeline,Realtime}` 真 enum；封死點是 `crates/interaction-session/src/session.rs:469` 的 HostOffer 只放 `Semantic`（撰稿時實讀兩處）。結論不變、理由要改寫 |
| 4 | v0.6.0 recovery matrix §2.7 #11（L-71） | agent principal 對 `POST /v1/memory` 的 403「需在 v0.6.0 補」 | `agent_request_allowed` 是 default-deny 且對所有 method 生效，POST 白名單不含 `/v1/memory` 與 `/v1/memory/context-bundle`——是 by construction 拒絕。缺的只是具名回歸測試 |
| 5 | `docs/aip/deprecation-ledger.md` §3.1（L-69） | 宣告式裝置側的「沒送過 capability 就永遠收不到 aip」反向承諾「未見具名回歸」 | `send_aip` 先 `ensure_ready_generation()`、`admit_aip` 先 `handshake_ready()`，已結構性強制，且有不同名的元件測試。缺的只是一支同名端到端回歸 |
| 6 | 矩陣 L-16 自述 | iOS `URLSessionSocket` 的證據「only code review」 | `apps/interaction-ios/README.md:665-712` 記載 2026-09-04 曾以真 `URLSessionSocket` 對真 daemon 在 iOS 模擬器完成配對／協商／touch intent 走查。**那是一次性模擬器證據，不是可重跑的 repo 測試**，`not-run` 的結論不變 |

### 6.3 文件（或矩陣）敘述與程式不符，方向不是「修掉／新增」

| # | 出處 | 不符之處 | 依據 |
|---|---|---|---|
| 1 | `docs/aip/deprecation-ledger.md` §1.1（L-41） | 記 `migrate_legacy_pack`「只剩測試呼叫端」 | 實際連測試呼叫端都沒有：repo 內只有定義那一處（撰稿時 `grep -rn migrate_legacy_pack crates apps --include='*.rs'` → 1 行） |
| 2 | 矩陣 L-64 的 entryPath | 宣稱 `docs-claims.sh`「CI Tauri backend job 亦跑」 | `.github/workflows/` 對 `docs-claims`／`architecture-checks`／`drills` 零命中（撰稿時實跑 grep），唯一呼叫點是手動的 `scripts/release-verify.sh:100-105`。**「腳本可跑」與「CI 有保護」必須分開寫**（與 L-48 一致） |
| 3 | 矩陣 L-06 的 reproCommand | 指向 `gateway.rs:330-332` 的 SessionSpec 組裝含 `model` | 撰稿時 `grep -n model crates/interaction-runtime/src/{gateway,agents}.rs` **兩檔零命中**；`model` 欄位在 `crates/interaction-agent-gateway/src/lib.rs:166`（預設 `None`，`:189`），claude 連接器會轉發（`claude.rs:103-104`）。結論（runtime 從不設定 model）不變、引用要改 |
| 4 | 矩陣 L-24 的 existingTests | 把測試歸到 `declarative_session_loop.rs` | 實際在 `providers_loop.rs`（與唯一呼叫點同檔） |
| 5 | 矩陣 L-43 的 reproCommand | `pnpm test -- settingsTransfer` | 該測試檔名不存在；真正的覆蓋是 `companion-gateway-wiring.test.ts` 等 |

### 6.4 本輪實跑推翻既有敘述（必須改寫，否則之後的引用會錯）

| # | 既有敘述 | 本輪實測 | 依據 |
|---|---|---|---|
| 1 | L-02：claude 被 SIGINT 後「結局落成 `unknown`（或非零 exit 時 `failed`）」 | **5/5 都是 `failed`**，而且 close 之後 SSE 再壓成 `closed` | D1／D2；`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1/claude-interrupt/`、`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1/b07-claude/`、`docs/releases/evidence/2026-09-07-phase-0/real-agent-e2e-prior/{claude-cancel-1,multi-C-claude-x2,multi-E-claude-codex}/` |
| 2 | L-10：真 codex／claude 二進位對接「在 repo 自動化證據中零執行」 | 本輪已用真 Claude Code 2.1.263／Codex 0.153.4 跑完 E0-02～E0-10（**不是**經 `E2E_REAL_AGENTS=1`，而是 `scripts/tests/phase0/` 的 harness） | `docs/releases/evidence/2026-09-07-phase-0/analysis/e2e-baseline.json` 的 `table[]`；`docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1`–`R8` |
| 3 | L-03 與 L-60 併看 | runtime 層誠實（`Expired`＋一則 `unknown` 事件）**不等於**產品層誠實（卡片顯示「已到期／muted」、無 honesty 文案） | L-60 已由懷疑者改判 `confirmed-defect`；`workState.ts` 的 `expired` 條目沒有 honesty 欄位、`unknown` 有 |

## 7. lint 執行記錄

本文件要求的收尾檢查，在完稿當下實跑：

```
$ bash scripts/tests/architecture-checks.sh --docs
architecture-checks @ /Users/user/Workspace/claude-lab/adaptive-interaction

── docs ────────────────────────────────────────────────
  ✔ scripts/tests/docs-claims.sh — docs-claims: 212 passed / 0 failed
  ✔ scripts/tests/release-scripts.sh — release-scripts: 58 passed / 0 failed

── 摘要 ────────────────────────────────────────────────
  docs       PASS
  ts         SKIP 未指定 --ts
  rust       SKIP 未指定 --rust
  drill-lint SKIP 未指定 --drill-lint
  swift      SKIP 未指定 --swift；不以測試檔存在當成通過
  drills     SKIP 未指定 --drills；靜態 lint 不代表演練通過
architecture-checks: 所選組全部通過；5 組未選取（不是通過）
```

三件要一起記住的事：

1. **未選取 ≠ 通過。** 上面只跑了 `docs` 一組；`ts`／`rust`／`swift`／`drills`／`drill-lint` 五組是 SKIP。
   這五組在本輪基線各自跑過一次（`docs/releases/evidence/2026-09-07-phase-0/baseline/summary.jsonl` 的
   `arch-ts`／`arch-rust`／`arch-swift`／`arch-drills`／`arch-drill-lint`，皆 exit 0），但那是 HEAD `78dcda1` 當下、
   不含本輪新增的階段 0 文件。
2. **`docs-claims` 的通過條數會漂移。** 本文件開始撰寫時是 **188 passed / 0 failed**，完稿時是 **212 passed / 0 failed**
   ——期間有其他階段 0 文件同時落盤、既有 `README.md`／`docs/*.md` 也被同批修改（`git status` 可見）。
   條數本身不是穩定的引用值；**唯一該引用的是「0 failed」**。
3. **這個 lint 不在 CI 上。** 它只在手動的 `scripts/release-verify.sh:100-105` 被呼叫（見 §6.3 #2 與 L-48／L-64）。

## 附錄 A：completeness critic 提出的漏列（17 條，**未逐列核實**）

> 這些是 dim-L 的 completeness critic 認為「71 列沒蓋到、但應該有一列」的主題。
> **本文件沒有對它們做 find→verify**，所以一律標「completeness critic 提出，未逐列核實」，
> 不得當成已確認的限制或缺陷引用。唯一的例外是 #15，本文件已獨立核對並升格到 §6.1 #5。
> 每條附 critic 自己給的 `whereToLook`（同樣未經核實）。

| # | 主題（critic 的說法） | critic 自估狀態 | 何處查（critic 提供，未核實） |
|---|---|---|---|
| 1 | **串流**：agent 進行中的輸出到不了使用者；`TaskProgress{text}` 只進 receptor inference，狀態事件 payload 無 text，信箱只收 result 與 approval-request | implemented-partial | `gateway.rs:407-412,456-500`；`agents.rs:1098-1106,1173-1250`；`AiPage.tsx:212-222,528` |
| 2 | **「一次一輪回 Busy」只對 codex_exec fallback 成立**：`GatewayError::Busy` 在 production 只由 `codex_exec.rs:140` 產生；claude 與 codex app-server 都不檢查進行中的一輪 | 與 L-59 衝突 | `claude.rs:323-341`；`codex.rs:604-621`；`codex_exec.rs:130-143`；`gateway.rs:660-670,759-761,790-800` |
| 3 | **提問（AI 問人、人回答）沒有端到端路徑**：信箱 kind `question` 只有桌面文案，runtime／三個 connector／Skill 都不產生也不教 | defined-only | `AiPage.tsx:221`；`skills/orchestrate-adaptive-interaction/references/api.md:101-108`；`lib.rs:89-99` |
| 4 | **iPhone 入口沒有任何 agent 工作面**：只有連線／感測／角色三個 tab | absent | `apps/interaction-ios/InteractionCompanion/Views/ContentView.swift:12-46` |
| 5 | **跨裝置核准零接線**：AIP 的 `MessageType::ApprovalRequest` 在 session／runtime／桌面／iOS 全部零引用 | defined-only | `crates/interaction-aip/src/{message.rs,envelope.rs,offline.rs}` |
| 6 | **AI 沒有任何讀寫記憶的正式路徑**：canonical tools 12 支不含 memory；agent 白名單排除 `/v1/memory` | absent | `interaction-tool-schema/src/lib.rs:625-641`；`interaction-api/src/lib.rs:488-541` |
| 7 | **記憶還原是前端逐筆重建**：沒有後端 import 端點、沒有去重、沒有單一交易或稽核 | implemented-partial（重複匯入無防護可能單獨構成缺陷） | `BackupSection.tsx:52-92`；`interaction-api/src/lib.rs:191-204` |
| 8 | **稽核可查詢性與保留策略**：`GET /v1/audit` 上限 500、無過濾無分頁；storage 對 audit 無刪除／保留策略 | implemented-partial | `interaction-storage/src/lib.rs:128,967-985`；`routes.rs:1036-1044` |
| 9 | **多 Agent／多 session 的實際上限**（`max_sessions` 預設 8、`max_parallel`）沒有任何一列記錄，桌面撞上限的文案未查 | 後端 implemented-and-connected ＋ 桌面 needs-investigation | `agents.rs:486-501`；`interaction-core/src/agent.rs:126,137,171-175` |
| 10 | v0.7.0 §4.2（清 marker 是第三次寫入）與 §4.3（交易中不下「半套用」判決）兩條未列入 | implemented-partial（by-design） | `v0.7.0-known-limitations.md:197-198` |
| 11 | v0.7.0 §3.8（`sensor_clock::advance()` 是 `#[doc(hidden)]` 的公開 API）未列入 | implemented-partial（by-design） | `v0.7.0-known-limitations.md:189` |
| 12 | v0.7.0 §1.7（native 走查用 debug build、結果不入版控）未列入；而 native-desktop 這一級證據被多列引用 | implemented-partial（影響可追溯性） | `v0.7.0-known-limitations.md:157` |
| 13 | deprecation-ledger §3.3（`reason:"recovery"` 新增值會讓未實作規則 6 的新接收端靜默降級）是 ledger 裡唯一沒有對應列的條目 | implemented-partial（by-design） | `docs/aip/deprecation-ledger.md:144-155` |
| 14 | v0.7.0 §6.1（`SemanticState` 不在跨語言 schema）被 v0.8.0 宣稱處理，但 71 列既沒列成已修也沒列成殘留 | implemented-and-connected（待補一列固定證據） | `v0.7.0-known-limitations.md:221`；`schemas/semantic-state-1.0.schema.json` 等 |
| 15 | `docs/acceptance-evidence.md` 末段仍記「`release.yml` 的 ci-gate／finalize 未經真實 tag push 驗證」，但 v0.8.0 已真的打 tag 發佈 | needs-investigation | **本文件已獨立核對並升格至 §6.1 #5** |
| 16 | 安裝平台的逐項缺口（x86_64 macOS 與其他平台的安裝驗證）未列出，L-47 只涵蓋簽章／公證／SBOM／Linux aarch64 | needs-investigation | `v0.8.0-known-limitations.md:21`；`docs/releases/v0.8.0-publication.md` |
| 17 | v0.7.0 §5.1（TDD 只做到一半）與 §5.3（`onMain` 同步執行未在真裝置量過）兩條未列入 | implemented-partial | `v0.7.0-known-limitations.md:212,214` |

## 附錄 B：completeness critic 提出的矩陣內部矛盾（16 條）

> 與附錄 A 不同的是，其中多條**已被獨立懷疑者在 §3 的改判裡證實**；那幾條在下表標「§3 已證實」。
> 沒有標的，一律是「completeness critic 提出，未逐列核實」。

| # | 矛盾 | 本文件的處置 |
|---|---|---|
| 1 | L-59 把「同 session 一次一輪回 Busy」寫成全域 by-design，但 `GatewayError::Busy` 在 production 只由 `codex_exec.rs:140` 產生 | **critic 提出，未逐列核實**。撰稿時 grep 確認 `GatewayError::Busy(` 在 `crates/interaction-agent-gateway/src/` 只有兩處：`codex.rs:966`（測試重試迴圈內）與 `codex_exec.rs:140`。L-59 的 verdict 為 `refuted:false`，故 §2 維持原狀態，但這條足以要求後續階段重驗 |
| 2 | L-64 說 `docs-claims.sh` CI 有跑、L-48 說 CI 只跑 cargo／pnpm／playwright | **§3 已證實**（L-64 改判理由）；並列入 §6.3 #2 |
| 3 | L-03（runtime 誠實）與 L-60（產品層不誠實）不能同時當結論 | **§3 已證實**（L-60 改判為 confirmed-defect）；並列入 §6.4 #3 |
| 4 | L-21 說 tray 會反映 unresolvedStops、L-37 說 tray 只週期更新 | **§3 已證實**（L-37 改判為 implemented-partial：embedded 事件驅動、external 只有輪詢） |
| 5 | L-71 的 contract 寫「記憶寫入須走 canonical tools」，但 canonical 12 支沒有 memory | **critic 提出，未逐列核實**（與附錄 A #6 同一主題） |
| 6 | L-41 列內自相矛盾（entryPath 說無 production 呼叫端、status 卻是 implemented-partial） | **§3 已證實**（改判為 defined-only） |
| 7 | L-62 的 grep 用小寫，必然零命中＝假陰性 | **§3 已證實**；並列入 §6.2 #3 |
| 8 | L-57「無下游消費者」不成立 | **§3 已證實**；並列入 §6.2 #1 |
| 9 | L-58 列內自相矛盾（limitation 說「native 走查有」，但腳本自記 `humanDismissal=False`） | **§3 已證實** |
| 10 | evidenceLevel 用字系統性不一致：in-process trait mock 被標成 simulator／browser／native-desktop，真 pty 子程序也標 simulator | **critic 提出**；懷疑者已在 L-04／L-13／L-25／L-27／L-32／L-70 各自更正。critic 建議的定義（unit＝純函式／trait mock；fixture＝程序內替身；simulator＝真子程序模擬器；integration＝真 HTTP＋真 Runtime）**本文件未採用為規範**，只照抄改判後的值 |
| 11 | `needs-investigation` 含意分兩種（缺環境 vs 連程式行為都沒查清） | **critic 提出**；懷疑者已把 L-29 改判為 implemented-and-connected。剩下的 6 列 needs-investigation 仍混用兩種語意 |
| 12 | L-07／L-08 的 owner 與實際強制點不符 | **§3 已證實**（兩列的改判理由） |
| 13 | L-02 的 verdict 自身標 `refuted:false` 但內文含 REFUTED CITATION（`work-delegate.spec.ts:249` 的 kind 其實對映到 codex，不是 claude） | **critic 提出，未逐列核實**。結論不受影響（L-02 已由 D1 以真 agent 5/5 重現）；引用應改為 `scripts/tests/tauri-work-cancel.py` 那條路徑 |
| 14 | L-67 方向相反 | **§3 已證實**；並列入 §6.1 #4 |
| 15 | L-16 低估既有證據 | **§3 已證實**；並列入 §6.2 #6 |
| 16 | L-14 把 `real-iphone` 與 `not-run` 併在同一列，易被引用成「AIP 有真機證據」 | **critic 提出，未逐列核實**。本文件在 §2 的 L-14 已明寫「本輪未取得真機證據」，並在 §5 B1 列出擋住它的環境條件；**本輪 real-iphone = 0 筆** |

## 附錄 C：相關文件

- 入口與續接：`docs/releases/phase-0-progress.md`
- Repository 真實狀態：`docs/releases/phase-0-repository-state.md`
- 能力恢復矩陣（維度 A–L 的完整盤點）：`docs/releases/phase-0-capability-recovery-matrix.md`
- 文件與現況落差：`docs/releases/phase-0-docs-vs-reality.md`
- 架構現況：`docs/releases/phase-0-architecture.md`；測試責任分布：`docs/releases/phase-0-test-responsibility.md`；後續路線：`docs/releases/phase-0-roadmap.md`
- 真 agent／多 session E2E 基線（E0-01～E0-11 的完整判讀）：預定為 `docs/releases/phase-0-e2e-baseline.md`
  ——`scripts/tests/phase0/README.md` 已指向此檔名，但**撰稿當下該檔尚未落盤**；在它出現之前，E0-xx 的原始判讀請直接讀
  `docs/releases/evidence/2026-09-07-phase-0/analysis/e2e-baseline.json`
- 原始證據與 manifest：`docs/releases/evidence/2026-09-07-phase-0/`（`docs/releases/evidence/2026-09-07-phase-0/artifact-manifest.json` 記錄每一份檔案的原始路徑、sha256 與正規化方式）
