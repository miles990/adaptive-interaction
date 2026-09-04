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
