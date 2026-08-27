# v0.4 差距分析（Phase 0）

基準：`main` @ `18037e5`（v0.3.0 + docs）。
回歸基線（2026-08-27 實測）：Rust workspace **201/201**、vitest **42/42**（+2 出貨
pack 驗證）、Playwright 瀏覽器 E2E **11/11**、CLI E2E **12/12**。

> 本文前半是 Phase 0 當時快照，不回寫歷史。Closing audit 見下方。

## Closing audit（2026-08-28）

Phase 0 的歷史快照保留在下方。最終實作與獨立對抗審查收斂後，嚴格 Capability
Matrix 為 **25/25 complete、0 partial、0 missing**。本輪關閉：

- Presentation Provider／角色預設能力／小樞 v2／Behavior Runtime 與程序化視線耳朵疊加。
- 五種主動對話政策、`behaviorIntent` 白名單、Codex app-server＋exec fallback、
  Claude stream-json／resume／限權寫入、Gateway／Cancel／Approval／Process tree。
- Human／Agent token 分權，agent 子程序不繼承 Runtime token。
- 10 層記憶、CAS、Knowledge Graph／FTS5／Candidate workflow、更新決策、
  使用者糾正入口、Knowledge Receipt 與 Companion 固定文案。
- 17 類 metadata-only 硬體 discovery 模型，以及 macOS `system_profiler`、Linux stable
  by-id 的真實列舉；Runtime／API／CLI／Onboarding／控制中心閉環。
- 控制中心八個要求頁＋Automation 相容頁、Global Search／Command Palette、複合
  Activity filters、Consent／Receipt／圖像區域與音視訊時間段 Source Viewer。
- 生成式主動觸發器建立真實限權 Agent Session；Context Bundle 自動附上並保存；
  本機 sparse subword embedding；thumbnail／WAV features 與可選 OCR/whisper/ffmpeg；
  Session/Domain token；100 張 desktop／390px 狀態矩陣。

逐欄 N/A 理由與保留限制見 `docs/capability-completion-matrix.md`；完整測試與畫面
證據見 `docs/v04-final-machine-evidence.md`。

## Phase 0 已修

- **出貨 persona pack 整包失效**（v0.3 回歸）：對抗審查後把 `succeeded-verified`
  列入 SAFETY_KEYS，但出貨的 `persona-shu.json`／`persona-navigator.json` 仍含該
  鍵 → `validatePersonaPack` 拒收 → CompanionApp 靜默退回 DEFAULT_LINES。
  已移除該鍵並新增「出貨 pack 必須通過驗證器」測試（packs.test.ts）。

## 差距總表（spec 章節 → 現況 → 需要）

| Spec | 現況（v0.3） | 差距 |
|---|---|---|
| §2 Provider 統一模型 | ProviderKind 含 Companion/AiAgent/AiSession；生命週期禁捷徑已實作 | 硬體掃描 adapter 介面與發現模型全缺；掃描不啟動感測的測試缺 |
| §2.2 Presentation Provider | 小樞是純前端視窗，互動走語意 push；**未註冊為 provider**，能力未逐項化 | 新增 companion provider＋逐項 receptor/actuator＋consent＋presentation command 執行迴路（SSE→視窗→ack→receipt） |
| §3 角色能力預設 | 無逐項開關；點擊/拖放/文字輸入永遠開 | 逐項預設矩陣（低風險自動開、敏感另行確認）＋隱藏角色時停用角色 receptor |
| §4 角色重設計 | 2-2.5 頭身、深灰藍、耳朵語意色 | 3–3.5 頭身貓系數位小精靈、眉眼表演、尾巴慣性、靈巧/慵懶/活潑三變體、failed 專屬美術（現況借用 blocked） |
| §5 Behavior Runtime | 500ms pose pump＋blink 定時器 | 三層系統：BehaviorState 平滑量、注意力鏈（眼→頭→身）、微動作疊加、Attention Manager＋Utility AI、反重複 |
| §6 主動/被動交互 | 無主動對話概念；氣泡 cooldown 常數 | 五種模式、每小時上限/最短間隔/合併/不追問/勿擾延後/安全去重、氣泡快速選項、非語言先行階梯 |
| §7 behaviorIntent | 無 | 結構化 Schema＋runtime 驗證 |
| §8 Agent Connector | agent session/mailbox 存在但**無真實 agent 接線** | codex app-server JSON-RPC connector（協定 schema 已鎖，0.149.1 支援）＋claude stream-json connector（已實測往返）＋exec fallback＋process tree 管理 |
| §8.3 Gateway 正規化 | EventType 無 agent.* 生命週期事件 | 正規化事件層＋Provider Session ID 映射 |
| §9 AI 交互設定 UI | 無 | Agent 連接/路由/對話/任務/記憶 五區設定頁 |
| §10 角色記憶分層 | HandoffSummary 是唯一雛形 | Persona/Character/User/World/Domain/Know-how/Skill/Task/Session 分層資料模型 |
| §11 多模態知識庫 | 無 | CAS 素材庫＋Knowledge Graph（Entity/Claim/Relation/Evidence/Provenance/Candidate）＋FTS5（bundled sqlite 已含）＋向量介面 |
| §12 知識 Tools | 無 | knowledge.search/get/propose-* 受限 tool＋Candidate-only 寫入＋Context Bundle |
| §13 更新決策 | 無 | 確定性 vs AI 更新分離＋觸發清單＋發布政策 |
| §14 經驗轉知識 | 無 | Observation→…→Validated Know-how 成熟度流程 |
| §15 保存期限 | observations 72h prune；其餘無 | expiresAt/reviewAfter/until-deleted 三態＋預設表＋deleteWithParent |
| §16 記憶管理 UI | 無 | 記憶與知識頁＋影響預覽 |
| §16-1 控制中心重構 | 一般 8 頁（home/senses/responses/toolops/automations/safety/activity/settings）＋進階 7 頁 | 新 IA 8 一級頁（首頁/小樞/AI 與工作階段/能力與裝置/記憶與知識/活動/隱私與安全/設定）＋Global Search＋Command Palette＋Activity Inbox＋Consent Sheet＋狀態元件 |
| §17 Knowledge Receipt | ActionReceipt 存在 | knowledgeReceipt 型別＋事件＋UI |
| §18 測試 | 201+42+11+12 | 每個新縱切的 unit/integration/E2E |

## 連接器可行性（已實測）

- `codex` 0.149.1：`app-server generate-json-schema` 產出完整協定
  （ClientRequest 95 方法、ServerNotification 75 通知，含 `thread/start`、
  `turn/start`、`turn/interrupt`、Exec/ApplyPatch/Permissions approval）。
  樣本存 scratchpad `connector-probe/codex-schema/`。
- `claude` 2.1.247：`--input-format stream-json --output-format stream-json
  --verbose` 真實往返成功（system/init 含 session_id → assistant →
  result/success 含 cost）；`claude auth status` 回 JSON（loggedIn:true）。

## 實作順序（依 spec §19，縱切完成制）

Phase 1 Presentation Provider＋角色重設計＋Behavior Runtime →
Phase 2 主動/被動模式＋behaviorIntent →
Phase 3 Agent Gateway＋雙 connector →
Phase 4 記憶模型＋保存期限＋Context Bundle →
Phase 5 素材庫＋Knowledge Graph＋Tools →
Phase 6 更新決策＋經驗轉知識＋Knowledge Receipt →
Phase 7 控制中心新 IA＋全域元件＋完整 E2E＋證據。

每 Phase：先測試 → 實作 → 跑測試 → 文件 → 證據；完成欄位同步更新
`capability-completion-matrix.md`，未完成不得標 complete。
