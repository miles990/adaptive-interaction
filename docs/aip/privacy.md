# AIP 隱私邊界：離開裝置的資料、留在本機的資料、稽核與保存期限

> 證據等級用字：本文件只寫「程式碼裡有這條防線」（附 `path:line` 或函式名）與「有沒有對應測試」
> （附測試名）；沒有測試的一律誠實標「未覆蓋」或「無測試斷言」。**不得**寫「已驗證／可運作」，
> 除非附上真正跑過的測試名；真機一律「未驗證」；fixture 一律標「模擬 iPhone（fixture）」。
> 這裡談的是 v0.6.0 新增的 AIP 語意層與 Character Session；既有的麥克風／記憶／稽核規則
> （`docs/ARCHITECTURE.md`「感測隱私」節）不變，本文件只補上 AIP 這一層新增的部分。

## 1. 離開裝置的資料（語意層，AIP 只同步這些）

依 `docs/aip/README.md` §0 設計原則 1：AIP 只同步「發生了什麼」「角色現在是什麼語意狀態」
「角色想表達什麼」「哪些能力可用」。

| 會離開裝置／跨 Transport 傳送 | 型別／欄位 | 來源 |
|---|---|---|
| 互動事件 | `character.interaction.touch{kind, intensity}`／`.dismiss{}` | `crates/interaction-session/src/director.rs`：`InteractionEvent`（只有 `kind`／`intensity`／已綁定身分／correlation） |
| 語意狀態 | `mood{kind,intensity}`／`activity`／`attention`／`truth{state,correlationId}`／`lastInteraction`／`members`／`reducedMotion` | `crates/interaction-session/src/state.rs::SemanticState`（`docs/aip/character-session.md` §3） |
| Behavior Intent | `intent`／`intensity`／`interruptible`／`origin`／`hints`（建議值，例如 `haptic:"light"`） | `crates/interaction-session/src/types.rs::BehaviorIntent` |
| 能力宣告 | `role`／`profiles`／`syncClasses`／`intents`／`inputs`／`features`／`limits` | `crates/interaction-aip/src/capability.rs::CapabilityAnnouncement` |
| 真相事實（唯 Runtime 可送） | `task.state{truth,correlationId}`／`task.verified{correlationId}`／`runtime.emergency{engaged}` | `crates/interaction-session/src/types.rs::RuntimeFact`；只能經 `Runtime::character_session_submit_runtime` 進來 |

## 2. 不會離開裝置的資料

| 不同步 | 理由／防線 |
|---|---|
| 影格（frame）、粒子、FPS | AIP §0 設計原則 1 明文排除；`SemanticState` 沒有這些欄位（`state.rs`），無法序列化出去 |
| 原始游標座標、絕對螢幕座標 | 承襲 CPP §6 既有規則（「不保存原始游標軌跡、不送 AI」）；AIP 的 `character.interaction.touch` payload 只有 `kind`／`intensity`，`crates/interaction-session/src/director.rs::InteractionEvent` 結構上沒有座標欄位可裝 |
| 原始感測資料（iPhone motion／battery／mic-level 的連續數值流） | AIP 不承載這些——它們走既有 v1 `observation{receptor}` 路徑（`crates/interaction-runtime/src/mobile.rs`），AIP 只在其上疊加語意觸摸事件；iPhone 的 `character.interaction.touch` 是使用者「碰角色」這個語意動作，不是原始加速度計串流 |
| 麥克風原始音訊 | 不變（`docs/ARCHITECTURE.md`「感測隱私」）：只留 level 事實，不存不傳；AIP 未新增任何麥克風相關 message name |
| Runtime token、agent token、裝置配對 token 明文 | AIP envelope 沒有攜帶任何 token 的欄位；身分靠 Transport 綁定（`docs/aip/README.md` §5），`source` 只是宣稱後比對，token 本身不上 AIP wire |

## 3. iPhone 原始感測資料：本機處理，AIP 只看得到觸摸語意

iPhone 端的動作分類（例如把加速度計數據判定成「拿起手機」）發生在 App 本機
（`InteractionCompanionTests/MotionClassifierTests` 等既有測試，證據等級：模擬器，iPhone 17 模擬器，
非本文件新增範圍）。AIP 1.0 沒有新增任何把原始 motion／battery／mic-level 數值送上 `aip` frame 的
message name（`docs/aip/README.md` §2.3 的 `character.interaction.*`／`character.behavior.*`／
`character.session.*`／`task.*`／`runtime.*`／`device.*` 六個命名空間裡沒有原始感測數值的位置）；
`device.*` 命名空間在 1.0 保留給 Device Profile（`docs/aip/device-profile.md`），但**未定義任何 payload
形狀**，目前沒有實作可以把感測數值塞進去。

## 4. Diagnostics：宣稱與程式碼

`docs/aip/character-session.md` §10 與 `docs/aip/transport-bindings.md` §2 都宣稱
`GET /v1/character-session/diagnostics` 不含 token、路徑、原始 payload。核對
`crates/interaction-runtime/src/character_session.rs::character_session_diagnostics_value`
（:327-351）的輸出欄位：`sessionId`／`sessionEpoch`／`revision`／`sequence`／`members`（`party`／
`role`／`presence`／`lastSeenAt`）／`counters`／`eventLog{len,cap}`／`storeNote`。

- `party` 是 `{kind, id}`（例如 `device:iphone-87b42264`）——`id` 是配對時隨機產生的 device id
  （`format!("iphone-{}", &token_hex(8)[..8])`，`crates/interaction-runtime/src/mobile.rs`），
  不是 token、不是可逆推的識別碼；純函式層測試：
  `crates/interaction-session/tests/session.rs::diagnostics_counts_without_leaking`(:1105)。
- **`storeNote` 是固定文字（v0.6.0 已修，`b5cdfa2`）**：`character_session.rs::CharacterSessionHost::open`
  在快照讀不到／壞掉時只回 `STORE_NOTE_UNUSABLE`／`STORE_NOTE_UNREADABLE` 兩個常數之一，底層
  `PortError`／反序列化錯誤只進 tracing log，不進 API 回應。回歸測試
  `character_session::tests::store_note_never_carries_error_details_or_paths` 以含路徑樣式與
  「secret-looking-content」的壞檔啟動 host，斷言 note 不含檔案內容、不含暫存目錄路徑、不含任何
  插值的錯誤細節，且壞檔被隔離為 `.corrupt`、epoch 從救回的值 +1。

## 5. 稽核紀錄

AIP／Session 層寫入的稽核種類（`crates/interaction-session/src/session.rs` 常數區，
:39-45 一帶）：`aip.rejected`、`aip.identity-mismatch`、`aip.duplicate`、`character.session.join`、
`character.session.leave`、`character.session.presence`、`character.session.applied`、
`character.session.truth`、`character.session.emergency`、`character.session.intent-expired`、
`character.session.intent-dropped`、`character.session.cancel`。這些是 `Output::Audit{kind, detail}`
純函式輸出，由 Runtime 端的 `Store::audit(kind, actor, detail)`
（`crates/interaction-runtime/src/character_session.rs` 的 dispatch 邏輯）寫入既有 SQLite audit 表——
與 Runtime 其他子系統共用同一個 sink 與同一份既有稽核基礎設施，**沒有**為 AIP 另開一份稽核儲存。

`detail` 的內容只含固定鍵與已正規化的值（`safe_name(&envelope.name)`、`ErrorCode`、`party`），
不含 payload 原文；這與 `docs/aip/README.md` §5「錯誤訊息不回顯輸入」的規則一致
（`crates/interaction-aip/src/envelope.rs::sanitize_serde_error`(:303) 與
`ErrorPayload::new`（`error.rs`）的 200 字截斷同樣適用於 wire 上的 `error` envelope，
但稽核 `detail` 本身是否逐欄位排除所有可控字串，本次未逐一核對每個 audit call site，
標記為「部分核實」而非「已驗證」）。

## 5.1 未解決停止：跨重啟仍然未知（本輪 N3）

Canonical owner 是 Runtime `sensor_journal.rs::SensorJournal`，停止協調仍在
`sensors.rs::Runtime::stop_all_sensor_sources`。即時 `activeSensors`、未解決摘要
`unresolvedStops`、SQLite 歷史 audit 是三個不同面向。來源移除後的 60 秒即時窗口
只影響投影；跨 process 恢復的記錄直接進未解決摘要，**不冒充當下擷取**。

儲存使用現有 SQLite `meta.sensor_stop_journal`，獨立 `format: 1`，不是 AIP wire、
SemanticState profile 或 Character Session snapshot format。每筆只持久化 source ID、
process incarnation、來源 generation、感測種類與 adapter 提供的連線 scope、時間、原因、人話名稱及
`confirmedStopped: false`；不保存音訊、觀測內容、`startedBy`、`purpose` 或完整 `lastKnown`。
因此恢復後 `lastKnown` 為空陣列，不能把它當成重新取得的裝置觀測。

- Runtime 啟動時先在 `sensor_source_generation_high_water` 原子預留一段 generation，
  來源登記使用該段、不重用上一個 process 的 ID。預留值與 JSON generation 都有 2^53−1 上限；
  預留失敗或序號 metadata 損壞時拒絕啟動，不以不可信世代啟用感測。
- 停止遠端來源前、移除或覆蓋來源登記前，先保存最少必要的 unknown。
  `scoped_captures` 有 500 ms 上限；來源不回覆時以既有高風險能力宣告形成 unknown。
  摘要的來源查詢並行、有界；只排除目前 live 清單仍表示的同 scope 與感測種類，
  或仍在 60 秒孤兒窗口的紀錄。家族來源常駐不代表已離線的成員仍可見。
- Mobile 在原始 `micLevel` 從 false 變 true 時，先經 canonical capture port 保存，
  再發布該 connection 的擷取狀態；不依賴 receptor enabled、consent 或先按全域停止。
  connection 攜帶登記時取得的 source owner；重複 true 心跳不重寫，只有同 connection
  scope 且來源登記世代仍有效的明確 false 才確認解除。來源正被替換時保守保留，等後續有效證據。
  斷線、撤銷、被新 socket 取代與程序重啟都不確認停止。
  disabled 手機自報仍在擷取時，未解決摘要可見；這不會啟用受器或授予 consent。
  若來源 owner 不存在，保存有界 overflow 提醒，不能靜默遺失未知。
- 摘要最多 32 筆，每筆 `(scope, sensor)` 證據最多 64 組，整份 JSON 最多 256 KiB。
  滿載或資料無法逐筆保存時，保留既有明細與持久化 overflow 標記，另寫 audit；
  `overflowCount = 1` 表示**至少一組無法逐筆列出的 unknown**，不是準確的漏列筆數。
  解除所有可見明細也不能使 overflow 消失；查 audit 與實際裝置後才能制定復原操作。
- 同一個 SQLite transaction 保存提醒增刪與解除 audit；失敗回滾解除，保留記憶體提醒，
  回應失敗而非假成功。一般摘要寫入失敗仍繼續必要的實體停止／來源清理，
  `unresolvedStopHealth.storage = write-failed` 與 `recoveryUnknown` 保持可見。
- 停止回覆只可解除**送出請求時捕獲、且收到回覆時仍有效**的同 source generation，
  並且 process incarnation、capture scope 與 sensor 相同。Mobile 的 scope 取自既有
  connection ID，在擷取與停止出站快照中分別捕獲；同 device ID 重連也不能背書舊連線。
  來源預設 scope 是 source ID，多裝置 adapter 沒有提供成員證據時採保守保留。
  新的擷取清單只追加未知集合，不會抹掉舊未知。`stopped`、有 `confirmedVia` 的 `already-stopped`
  才有停止證據；Mobile 同一條有效連線的明確停止自報也可解除自己的 scope。
  transport write、空本機旗標、新來源相同 ID、晚到已失效連線的回覆都不能替舊記錄作證。
- 人類 `dismiss` 只表示已檢查並解除提醒，回應和同交易 audit 永遠
  `confirmedStopped: false`。同一家族仍有 live 成員時，只解除畫面列出的歷史 scope，
  不刪掉 live 成員的預先保存提醒。AI token 無法呼叫解除路徑；跨重啟也不變成裝置確認。
- 格式損壞、過大、future format 會 parked，原始 metadata 不改名、不隔離、不覆寫；
  新來源仍保留停止通道，記憶體 unknown 與 health 對外可見。序號預留 metadata 是
  獨立欄位，因此不必改寫 future document 也能避免 process 間世代復用。
  此機制不讀寫 Character Session snapshot／epoch，也不更動其 future-format 保護。
- 非乾淨重啟無法證明最後一次寫入成功，因此即使 itemized list 為空，仍顯示
  `recoveryUnknown`；這是保存完整性未知，不是「現在還在感測」。正常 shutdown 先走
  既有有界 sensor stop，再取消 transport，journal flush 成功才標記 clean。
  遠端等待上限沿用 `STOP_SENSORS_WAIT` + 500 ms source grace，加至多 500 ms capture 查詢；
  來源並行，本機麥克風先停止。SQLite/OS I/O 延遲不計入 transport deadline。

`status`、HTTP `GET /v1/sensors/unresolved`、CLI `sensors unresolved`、內嵌 Tauri 都讀
同一 application projection；一般模式、tray 同時處理 itemized unknown 與保存／overflow
警示，不能在陣列空時說「沒有問題」。Agent status 只拿原有無識別碼計數與 health，
不會拿到 source ID、incarnation、generation、lastKnown。

相容行為差異：v0.7.0 允許同 ID 新來源確認來清舊世代；本輪因為 port 沒有可驗證的
跨連線硬體 incarnation 證明，收緊為同登記／同 process。舊版已遺失的 memory-only
摘要不能補造；不從 audit 自動推測或重播一份 unknown。範圍與回退見 deprecation ledger §4.3。

回歸入口：`sensors_loop.rs` 的 `restart_*`、`new_generation_confirmed_*`、
`journal_dismissal_commit_failure_*`、`shutdown_persists_unknown_*`；
`tests/sensor_journal_review.rs` 的集合保留、同 kind 成員／同 ID 新連線證據隔離與 live family 可見性；
`mobile_loop.rs` 的單機 stop／direct revoke、disabled raw capture、natural disconnect、
supersede 與同連線 false-confirm 跨重啟回歸（TLS 模擬器）；
API `sensor_recovery_health_is_public_but_incarnations_are_human_only`；TS
`unresolvedStops.test.tsx`；Tauri `unresolved_storage_and_overflow_are_visible_without_active_capture`。
以上是測試入口，不代表真機驗收；本輪命令、數量與證據 SHA 由 progress/evidence index 記錄。

## 6. 保存期限

- **Memory Provider 分層**（`crates/interaction-core/src/memory.rs::default_retention`）的三態
  （Active／Stale／Expired）與各層預設天數**不適用於** AIP session 狀態——`SemanticState` 不經過
  `MemoryLayer`，兩者是不同的資料模型（`docs/aip/character-session.md` §2 State Ownership：
  「長期記憶」的 canonical owner 是 Memory Provider，Session 明確「不碰」）。
- **AIP session snapshot**（`<home>/state/character-session.json`，
  `crates/interaction-runtime/src/character_session.rs::SESSION_STORE_FILE`）**沒有找到任何以天數
  為單位的自動清除機制**：新的 snapshot 覆蓋舊的（每 32 個 revision 或 60 秒，`persist_every_revisions`／
  `persist_interval_ms`，`crates/interaction-session/src/types.rs`:59-61），檔案本身會一直保留到
  下次覆寫或被使用者手動刪除；壞檔案會被改名為 `.corrupt` 隔離（`quarantine()`，
  `character_session.rs`:90-96）但**不會被刪除**。這點本次未在既有文件（README／FEATURES／
  known-limitations）看到揭露，屬本文件新記錄的觀察，不是重新確認既有宣稱。
- **AIP session epoch 檔**（`<home>/state/character-session.epoch`，
  `crates/interaction-runtime/src/character_session.rs::SESSION_EPOCH_FILE`，權限 `0600`）：
  只有一個十進位整數（`sessionEpoch`），**沒有任何個人資料、裝置識別碼或互動內容**。
  它的用途是「這台電腦曾經以多大的 epoch 跑過 session」——快照被隔離（`.corrupt`）或整個被清空時，
  靠它保證新 epoch 一定大於成員記得的值，成員才會在 resume 時拿到 `session-reset` 而不是把
  host 的新狀態當成 rollback 忽略。**可以與快照一起刪除**（兩個檔案一起刪最乾淨）；
  刪除之後 epoch 從**牆鐘秒數**（unix 秒）重新起跳，那個值保證大於任何以 1 起算的遞增 epoch，
  所以舊裝置重連時仍會正確地拿到 `session-reset`。與 snapshot 一樣沒有以天數為單位的自動清除。
- **有界事件日誌**（`EVENT_LOG_RING=512`，`crates/interaction-aip/src/limits.rs`）是容量上限，
  不是時間上限——舊事件因為環滿而被淘汰（`crates/interaction-session/src/session.rs::EventLog::push`
  :1454），不是因為到期。
- **稽核記錄**：本次搜尋 `crates/interaction-storage`／`crates/interaction-core` 沒有找到 audit 表的
  保存期限常數（`rg -i "audit.*retention|AUDIT_RETENTION"` 零命中）——誠實記錄為
  **未盤點／看起來沒有自動清除**，而不是「不保留」或「有保留期」。這是既有系統的既有行為，
  不是 v0.6.0 新增的缺口。

## 7. `claimed ≠ verified` 對隱私的意義

`verified` 只能經人類驗證路徑（`verify_agent_session`）產生，AIP 層再收斂一次：
`crates/interaction-session/src/session.rs::gate`（:690-693，第 8 步）拒絕任何 member 送來的
`status:"verified"`；`crates/interaction-aip/src/outcome.rs::Outcome::is_runtime_only`（:73）
只有 `Verified` 回 true。這與隱私的關係是：**外部 adapter／裝置永遠無法讓稽核或 UI 誤記一筆
「已由人類驗證」的事件**——如果允許偽造 `verified`，就等於允許沒有實際發生的人類行為被永久記錄
成看起來像真的審核紀錄，這本身是一種資料完整性（因而也是稽核可信度）問題，不只是功能正確性問題。
測試：`crates/interaction-session/tests/security_matrix.rs::every_result_envelope_validates_and_never_claims_verified`
(:260)、`crates/interaction-session/tests/session.rs::devices_may_not_produce_runtime_truth_or_verified`
(:531，**勘誤**：本文件先前誤植為 `pure_functions.rs`，該檔僅 456 行，531 行不存在於其中；已核實此函式實際
定義在 `session.rs`)、`crates/interaction-runtime/tests/character_session_loop.rs::human_verified_celebrates_on_the_phone_without_double_playing_on_the_desktop`
(:670，端到端，模擬 iPhone（fixture））。
