# Capability Completion Matrix（v0.4 歷史文件）

> **已被 v0.5 取代**：本矩陣衡量的是 v0.4 治理平台的完成度（25/25），不代表角色遊戲性、
> 真實硬體與 iPhone 的完成度。v0.5 的誠實基線與收尾請看 `docs/v05-capability-gap-matrix.md`
> 與 `docs/v05-recovery-matrix.md`。

更新：2026-08-28。唯一基準：本機 `main` @
`0aa8733ff8f5d7632d59a955a16c08cf1458a92e`（同步時 `origin/main` @
`75f913d9946c3221b7f47136755b2af08739713d`）加本輪未提交工程 diff。

`C` = complete；`N/A` = 該層沒有合理的執行語意，理由列在限制欄。必要欄位沒有
`partial` 或 `missing`：**25/25 complete**。詳細命令、真 Agent session、對抗審查、
畫面與 hash 見 `docs/v04-final-machine-evidence.md`。

| ID | Capability | Provider / Capability / Adapter | Runtime / Policy | Control Center / Companion / Tray | API / CLI | Receipt / Verification | Unit / Integration / E2E | 平台與限制 | Machine Evidence |
|---|---|---|---|---|---|---|---|---|---|
| BASE-001 | v0.3 Runtime、Governor、Consent、Estop 回歸 | C / C / C | C / C | C / C / C | C / C | C | C / C / C | macOS 實測；Linux CI；degraded fail-closed | Rust 336；CLI 51；PW 23 |
| DISC-001 | metadata-only 硬體掃描 | C / C / C | C / C | C / N/A（掃描非角色效果）/ N/A（無副作用可停止） | C / C | N/A（scan report＋audit，不造 ActionReceipt） | C / C / C | macOS `system_profiler`；Linux stable by-id；Windows 誠實 unsupported | `hardware_loop`、API scan test、scan PNG |
| PRES-001 | Presentation Provider 7 receptors＋7 actuators | C / C / C | C / C | C / C / C | C / C | C | C / C / C | ack 只升 AcknowledgedOnly，未獨立驗證不冒充 Verified | `presentation_loop`、CLI presentation |
| PRES-002 | 角色預設能力與 hide semantics | C / C / C | C / C | C / C / C | C / C | C | C / C / C | hide 停 surface receptor，Runtime／Tray／Agent 留存；hide≠estop | `hidden_companion_*`、PW companion |
| CHAR-001 | 小樞 v2 三變體 | C / N/A（資產非 capability）/ C | C / N/A（不增加權限） | C / C / N/A | N/A / N/A | N/A | C / C / C | 原創 SVG/sprite；v1 pack 僅相容保留 | `generate.mjs`、live companion PNG |
| BEH-001 | Behavior Runtime、Attention、Utility AI | C / C / C | C / C | C / C / N/A | C / C | N/A（純呈現決策） | C / C / C | 本機確定性；不保存游標軌跡 | `behavior.test.ts`、PW |
| PRO-001 | 主動對話五模式與背景 Agent 候選 | C / C / C | C / C | C / C / C | C / C | C | C / C / C | 真背景觸發建限權 session；預算、勿擾、合併、不追問 | `proactive_loop`、`ai_generated_recipe_creates_*` |
| PRO-002 | `behaviorIntent` schema／白名單 | C / C / C | C / C | C / C / N/A | C / C | C | C / C / C | 安全狀態與任意動畫名拒絕 | golden schema、whitelist tests |
| AGT-001 | Codex app-server＋exec fallback | C / C / C | C / C | C / C / C | C / C | C | C / C / C | Codex 0.150.1 真 app-server；fallback 真子程序 fixture | Session `asession-cd2b…` |
| AGT-002 | Claude Code stream-json／resume | C / C / C | C / C | C / C / C | C / C | C | C / C / C | Claude Code 2.1.247；寫入須 workdir＋二次確認 | Session `asession-8508…` |
| AGT-003 | Gateway events、Session、Cancel、Process tree | C / C / C | C / C | C / C / C | C / C | C | C / C / C | Unix process-group 實測；Windows tree 由介面測試 | `gateway_loop`、process tests |
| AUTH-001 | Human／Agent／Session-Domain token 分權 | C / C / C | C / C | C / N/A / C | C / C | C | C / C / C | 同 OS 帳號檔案隔離仍由 Agent sandbox 負責 | SSE restricted-event test、token E2E |
| UI-AI-001 | Agent 連接、路由、Session 控制 | C / C / C | C / C | C / C / C | C / C | C | C / C / C | create／approval／interrupt／續租／close；claim≠verified | regression Vitest、AI PNG |
| MEM-001 | 10 層記憶、期限、備份還原、刪除 | C / C / C | C / C | C / C / N/A | C / C | C | C / C / C | 還原逐筆經 Runtime 驗證並配新 ID；不是 raw DB 覆寫 | `memory_loop`、backup restore test |
| MEM-002 | 最小 Context Bundle | C / C / C | C / C | C / N/A / N/A | C / C | C | C / C / C | 每次真 task 自動附上並持久化實際 bundle | context bundle／gateway tests |
| KNOW-001 | CAS 多模態素材與衍生流程 | C / C / C | C / C | C / C / N/A | C / C | C | C / C / C | 本機 thumbnail／WAV features；可選工具缺少時明確 unavailable | assets、Source Viewer tests |
| KNOW-002 | Graph、FTS5、本機向量、Provenance | C / C / C | C / C | C / N/A / N/A | C / C | C | C / C / C | 稀疏 subword embedding 是真本機向量，非 neural 宣稱 | knowledge/fusion tests |
| KNOW-003 | 9 Knowledge Tools、Candidate-only 寫入 | C / C / C | C / C | C / N/A / N/A | C / C | C | C / C / C | Agent token 綁 session/domain；publish 只屬 human | schemas、API auth tests |
| KNOW-004 | update/freshness/conflict/supersede | C / C / C | C / C | C / C / N/A | C / C | C | C / C / C | 確定性步驟不呼叫 AI；外部研究先問 | curator、supersede tests |
| KNOW-005 | Experience→Candidate→Know-how | C / C / C | C / C | C / C / N/A | C / C | C | C / C / C | 單次偏好不普遍化；證據、反例、範圍必填 | curator tests |
| KNOW-006 | Knowledge Receipt／誠實角色文案 | C / C / C | C / C | C / C / N/A | C / C | C | C / C / C | persona 不可改寫六種安全文案 | knowledge receipt tests |
| UI-IA-001 | 9 一級頁、390px、鍵盤、狀態矩陣 | C / C / C | C / C | C / C / C | C / C | C | C / C / C | 8 要求頁＋v0.3 Automation 相容頁；offline 是 shared app state | PW 23；100 PNG |
| UI-GLOBAL-001 | Global Search／Command Palette | C / C / C | C / C | C / N/A / C | C / C | C | C / C / C | 僅列目前權限可執行命令；estop 二段確認 | global search E2E/Vitest |
| UI-ACT-001 | Activity Inbox／compound filters | C / C / C | C / C | C / C / C | C / C | C | C / C / C | Agent／裝置／Domain／狀態／時間共用 application service | Activity tests、CLI inbox |
| UI-VIEW-001 | Consent／Receipt／Source／impact viewers | C / C / C | C / C | C / C / N/A | C / C | C | C / C / C | 圖片區域、音視訊 segment、程式位置由真 preview payload 呈現 | Source viewer tests、PW |

## 共用垂直閉環

所有有副作用 capability 都走：Provider discovery/identity → manifest/schema → registry →
Governor/Consent/Lease → adapter/tool → event/status → Control Center/Companion/Tray →
Receipt/independent verification → API/CLI → unit/integration/E2E。純資產、純演算法與
metadata-only 掃描的 N/A 不代表缺線；它們分別由資產驗證、確定性演算法測試與 scan
report/audit 證明，且不偽造外部效果 Receipt。

## 保留的誠實限制

- OS 能看見什麼仍受 driver、權限、sandbox、配對與裝置占用限制；UI 只說「已偵測到目前可用裝置」。
- OCR、語音轉錄與影片關鍵影格使用本機可選工具；工具不存在時回報 `unavailable`，不以假資料補齊。
- Agent claim 只到 `claimed-completed`；只有獨立測試／hash／observation 才能升 `verified`。
- 子程序孤兒回收以 pgid＋程序身分檢查；無法唯一歸因時 fail-safe 放棄並記錄 warning。
- 本輪沒有 release、deploy 或 push；這是 repo 操作政策，不是 capability 缺口。
