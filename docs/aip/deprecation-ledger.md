# 相容路徑退場登記表（deprecation ledger）

> 這份表的存在理由：**每一條相容路徑都是一筆會被遺忘的債。** 有 `#[deprecated]` 屬性的只有一個，
> 其餘的相容路徑（舊 id 白名單、舊快照格式、線協定追加訊息、feature flag、舊路由折疊表）都只是
> 一段沒有標記、沒有到期日、也沒有人記得為什麼還在的程式碼。這裡把它們逐條登記，讓「什麼時候可以
> 刪掉它」變成一個**有證據可查的問題**，而不是一次憑印象的判斷。
>
> 版本演進規則與相容矩陣在 `docs/aip/compatibility.md`；本表是它 §3 的可操作版本，涵蓋範圍比 AIP wire
> 更廣（包含桌面偏好、快照檔、路由與 CLI／診斷欄位）。
>
> 誠實用字：「移除前需要的證據」欄寫的是**還沒取得**的證據。沒有取得就不能移除——不得以「應該沒人在用」
> 這種推測代替。真機／真板相關的證據目前一律為零，見 `docs/acceptance-evidence.md`。

## 0. 欄位定義

| 欄位 | 意思 |
|---|---|
| 為什麼存在 | 移除它會弄壞誰。不是「歷史因素」，要指名具體的舊使用者或舊資料 |
| 適用版本 | 從哪一版開始存在；哪些版本的資料／裝置需要它 |
| 移除前需要的證據 | 要看到什麼才敢刪。可執行的檢查優先於文件宣稱 |
| 資料遷移 | 使用者的資料怎麼從舊形狀走到新形狀（沒有資料就寫「無」） |
| 回退方式 | 移除之後發現弄壞了，怎麼退回去 |
| 下一檢查里程碑 | 什麼時候再來看這一列 |
| owner | 哪個 crate／模組是這條路徑的權威實作 |

本表登記 **12 條**相容路徑，其中 11 條有完整七欄表格。只有 §2.2（未來格式不隔離、不覆寫）刻意
用敘述登記：它不是「將被移除的相容路徑」，而是一條要一直留著的防呆規則，七欄裡的「移除前需要的
證據」對它沒有意義。§3.2（裝置線 v1.2 `aip-frag`）的實作已經合併，欄位與 file:line 已補齊。

## 1. Rust API

### 1.1 `migrate_legacy_pack`（唯一有 `#[deprecated]` 屬性的項目）

| 欄位 | 內容 |
|---|---|
| 為什麼存在 | 舊呼叫端（repo 外的 host、既有整合）以單一參數呼叫遷移函式。抽出小樞 crate 之後遷移需要 host 注入的 `MigrationRegistry`，換簽名會讓這些呼叫端**編譯失敗** |
| 適用版本 | v0.6.0 標記（`#[deprecated(since = "0.6.0", …)]`）；v0.5.x 以前的呼叫端 |
| 移除前需要的證據 | repo 內零呼叫（`rg -n 'migrate_legacy_pack' crates apps` 只剩定義與文件）；且已跨過至少一個公開 minor（`compatibility.md` §3 流程） |
| 資料遷移 | 無（純函式改名／換簽名）。行為等價：內部就是 `migrate_pack_to_manifest(json, &MigrationRegistry::with_core_migrators())` |
| 回退方式 | 重新加回 4 行 wrapper |
| 下一檢查里程碑 | 下一個 minor |
| owner | `crates/interaction-character/src/manifest.rs`（定義在 `#[deprecated(since = "0.6.0", …)]` 之後的 `pub fn migrate_legacy_pack`） |

### 1.2 `accept_state_with_epoch`／`accept_state`（薄包裝）

| 欄位 | 內容 |
|---|---|
| 為什麼存在 | 決策表（`crates/interaction-session/src/receive.rs::decide_receive`）落地之後，這兩個函式已經**只是包裝**：`accept_state_with_epoch` 逐條委派給決策表，`accept_state` 更把本地 epoch 當成訊息宣告的 epoch。它們的簽名帶不進連線世代、hash 核對與本地 sessionId，所以規則 0／1／9／14 對它們永遠不成立。既有呼叫端（含跨語言鏡射的文件敘述）還指名它們 |
| 適用版本 | v0.6.0 起；v0.6.x 起變成決策表的包裝 |
| 移除前需要的證據 | production 呼叫端全部改用 `decide_receive`（目前 repo 內只剩測試與 `character_session_loop.rs` 的一處斷言引用）；三端文件（`transport-bindings.md` §7、`iphone-companion.md`）不再以它作為鏡射對象 |
| 資料遷移 | 無 |
| 回退方式 | 兩個函式各 3 行，隨時可以加回 |
| 下一檢查里程碑 | 決策表接完三端之後的第一個 minor |
| owner | `crates/interaction-session/src/patch.rs`（`accept_state`／`accept_state_with_epoch`），權威為 `crates/interaction-session/src/receive.rs` |

### 1.3 `ports.rs` 的 `RendererPort`／`DevicePort`：**零實作者（experimental）**

| 欄位 | 內容 |
|---|---|
| 為什麼存在 | `docs/aip/architecture-boundaries.md` §2 的 ports 表把它們列為穩定介面，但 repo 內**沒有任何 `impl`**：`rg -n 'RendererPort\|DevicePort'` 在程式碼裡只命中 `crates/interaction-session/src/ports.rs` 的 trait 定義。實際的 renderer 是 TS `CharacterAdapter`，實際的裝置是 transport 層 |
| 適用版本 | v0.6.0 起 |
| 移除前需要的證據 | 二選一：(a) 有第一個 production 實作者 → 從 experimental 轉正；(b) 確認 renderer／device 的擴充點永遠在 TS／transport 層 → 刪除 trait 並修正架構文件。**在此之前不得因為「介面存在」就宣稱這條擴充點可用** |
| 資料遷移 | 無 |
| 回退方式 | trait 定義本身沒有行為，加回即可 |
| 下一檢查里程碑 | 下一個新增 renderer／device 種類的工作 |
| owner | `crates/interaction-session/src/ports.rs` |

## 2. 資料格式與資料遷移

### 2.1 快照容器格式 `format: 0 → 1`

| 欄位 | 內容 |
|---|---|
| 為什麼存在 | v0.6.0 寫出來的 `character-session.json` **沒有** `format` 鍵（讀成 0）。刪掉遷移路徑等於讓 v0.6.0 使用者的 session 在升級後被當成損毀而隔離＋重建 epoch（成員全掉、需要重新確認） |
| 適用版本 | `SNAPSHOT_FORMAT = 1`（`crates/interaction-session/src/types.rs`）；來源格式 0＝v0.6.0 |
| 移除前需要的證據 | 統計不到 format 0 的檔案這件事**無法**從 repo 證明（檔案在使用者機器上）。所以這一列的移除條件是「跨越一個明確的長支援期」而不是證據；在那之前只能加、不能減 |
| 資料遷移 | 有，且不重建 session：驗完整性 → 反序列化（`deny_unknown_fields`）→ 驗不變量 → 原檔備份成 `character-session.json.pre-format-<n>`（同一來源格式只留一份）→ 以現行格式落地。epoch 不變、成員不掉 |
| 回退方式 | 備份檔 `character-session.json.pre-format-0` 就是回退點：改回舊版本後直接改名回去 |
| 下一檢查里程碑 | 下一個 major |
| owner | `crates/interaction-runtime/src/character_session.rs`（`SESSION_BACKUP_SUFFIX`）；fixtures `crates/interaction-runtime/tests/fixtures/character-session/`；測試 `character_session_loop.rs::a_v0_6_0_snapshot_is_restored_and_migrated_to_the_current_format` |

### 2.2 未來格式（`format > SNAPSHOT_FORMAT`）不隔離、不覆寫

不是 deprecation，是**這條相容路徑的對稱面**，一起登記免得被當成 bug 修掉：舊版本讀到新版本寫的檔案時
`PortError::FutureFormat`，store 進 parked（唯讀）、以記憶體跑完這一輪。覆寫它等於替使用者做了降級決定。
owner：`crates/interaction-session/src/ports.rs`；測試 `character_session_loop.rs::a_future_format_snapshot_is_kept_untouched`。

### 2.3 8 個舊小樞家族 id 的設定匯入相容

| 欄位 | 內容 |
|---|---|
| 為什麼存在 | v0.4／v0.5 出貨的 8 個 pack id（`shu-maid`／`shu-maid-dusk`／`shu-maid-sakura`／`shu-agile`／`shu-lazy`／`shu-lively`／`shu-standard`／`shu-minimal`）匯出的設定檔會夾帶當時**全域共用**的說話風格／場景／使魔。角色專屬欄位改綁 adapter meta 之後，這些檔會被判成「這個欄位不屬於這個角色」而整份拒絕——使用者自己匯出的檔匯不回來 |
| 適用版本 | v0.4／v0.5 匯出的 `schemaVersion: 1` 檔案；寬容規則自 v0.6.x 起 |
| 移除前需要的證據 | 同 2.1：舊檔在使用者機器上，repo 無法證明沒有人有。移除條件是明確的長支援期＋在 UI 提供一次性轉檔 |
| 資料遷移 | 不是轉檔，是**誠實忽略**：只有「問得出目標角色的 adapter、但它沒宣告那一項」才忽略該欄位；非舊 id 一律拒絕，問不出 adapter 也一律拒絕（不猜） |
| 回退方式 | 常數是一個陣列，刪掉即回到嚴格模式 |
| 下一檢查里程碑 | 下一個 minor（順便確認 CPP §2.2「這 8 個 id 永遠可用」是否仍是契約） |
| owner | `apps/interaction-desktop/src/companion/settingsTransfer.ts`（`LEGACY_CHARACTER_IDS`、`legacyTolerant`） |

### 2.4 `LEGACY_ANCHORS`：舊 tab id → 五入口的折疊表

| 欄位 | 內容 |
|---|---|
| 為什麼存在 | tray 深連結、Runtime Inbox route、使用者書籤、GlobalSearch 結果仍會送出舊的 tab id（`ai`／`automations`／`capabilities`／`senses`／`responses`／`toolops`／`safety`／`memory`／`activity`／`settings`／`backup`／`manage`／`advanced-features`）。少了這張表，舊深連結會導到一個沒有標題的空頁 |
| 適用版本 | 一般模式五入口收斂（v0.5.x）之後至今 |
| 移除前需要的證據 | 全部舊 id 的產生端（tray、深連結、GlobalSearch、Inbox route）都改送新 id，且已跨過一個公開版本讓舊書籤自然淘汰 |
| 資料遷移 | 無（純路由折疊） |
| 回退方式 | 一張 13 列的 `Record<string, string>`，加回即可 |
| 下一檢查里程碑 | 下一次 IA 變更 |
| owner | `apps/interaction-desktop/src/routing.ts`（`LEGACY_ANCHORS`／`navAnchorFor`） |

## 3. 線協定（wire）

### 3.1 裝置線協定 v1.0 → v1.1（`aip` 追加訊息）

| 欄位 | 內容 |
|---|---|
| 為什麼存在 | 已經燒錄的參考韌體只認得 v1.0 的 `hello`／`pair`／`cmd`／`ack`／`err`／`state`。`aip` 是**追加**訊息：`proto` 仍為 1，舊韌體收到就忽略、舊 host 當未知訊息丟棄，所以不需要 major bump——代價是 host 端必須永遠容忍「對方從來不送 `aip`」 |
| 適用版本 | v1.0＝v0.5 起的裝置線；v1.1（`aip`）自 v0.6.x |
| 移除前需要的證據 | 不會移除（追加訊息沒有「移除」問題）。真正要登記的是**反向承諾**：沒送過 `capability` 的裝置永遠不會收到 `aip`。這條承諾在 iPhone 側有回歸測試（`crates/interaction-runtime/tests/mobile_loop.rs::a_legacy_phone_that_never_negotiates_receives_no_aip_frames`）；宣告式裝置側的對應保證來自傳輸層准入（`DeviceLink::admit_aip`），**未見同名的單一回歸測試** |
| 資料遷移 | 無 |
| 回退方式 | 停止送出 `HostMsg::Aip` |
| 下一檢查里程碑 | ESP32 真板取得第一筆證據時（目前為零） |
| owner | `crates/interaction-adapter-declarative/src/protocol.rs`（`DeviceMsg::Aip`／`HostMsg::Aip`／`admit_aip`／`MAX_AIP_ENVELOPE_BYTES`）；契約 `docs/aip/device-profile.md` §6 |

### 3.2 裝置線 v1.1 → v1.2（`aip-frag` 追加訊息）

分片訊息用來繞過參考韌體 639 bytes 的單行上限（`device-profile.md` §6 已知限制 (1)：協商的第二則
snapshot 回覆與含 `members` 的 patch 送不出去，只能稽核 `aip.outbound-undeliverable`）。實作已合併
（`9799b1e`），下表是它產生的相容承諾。

| 欄位 | 內容 |
|---|---|
| 為什麼存在 | 與 §3.1 同一個理由，方向相反：`aip-frag` 是 v1.2 的**追加**訊息，`proto` 仍為 1。已經燒錄的 v1.0／v1.1 韌體（含本 repo 的參考韌體）不認得它，收到必須**忽略而不是斷線**。代價是 host 端必須永遠容忍「對端不會重組」，並在那時誠實地一個位元組都不寫 |
| 適用版本 | v1.1＝v0.6.x 起的裝置線；v1.2（`aip-frag`）自 v0.7.0。參考韌體停在 v1.1（不宣告 `aip.frag/1`，沒有重組緩衝） |
| 移除前需要的證據 | 不會移除（追加訊息沒有「移除」問題）。要登記的是**反向承諾**：沒宣告 `aip.frag/1` 的裝置永遠不會收到 `aip-frag`。閘門在 `protocol.rs:769`（`DeviceLink::supports_fragmentation`，出站 `== Some(true)`）與 `protocol.rs:664`（`accept_fragment`，入站對稱地用 `!= Some(true)`——沒有宣告 caps 的舊韌體也一樣被拒，原因 `not-advertised`）；回歸測試 `crates/interaction-runtime/tests/declarative_session_loop.rs::a_device_without_fragmentation_degrades_to_intent_only`（`--no-frag` 的模擬器一個位元組都收不到、降級成 `intent-only`）與 `crates/interaction-adapter-declarative/tests/esp32_sim_conformance.rs::the_firmware_ignores_aip_frag_and_never_claims_it_can_reassemble`（韌體 `handleMessage()` 明確處理 `aip-frag`＝忽略，且 `hello.caps` 不含 `aip.frag/1`）。**真板未驗**：模擬器的忽略行為不是韌體的忽略行為 |
| 資料遷移 | 無 |
| 回退方式 | 讓 `supports_fragmentation()` 恆回 `false`（`protocol.rs:769`／`:1643`）：出站退回 v1.1 的「放不進就拒絕並稽核 `over-line-limit-no-fragmentation`」，入站的 `aip-frag` 一律不收。wire 上沒有需要撤回的東西 |
| 下一檢查里程碑 | ESP32 真板取得第一筆證據時（目前為零），或第一個非模擬器的第三方裝置宣告 `aip.frag/1` 時 |
| owner | `crates/interaction-adapter-declarative/src/fragment.rs`（`FRAG_CAP:31`／`MAX_REASSEMBLED_BYTES:35`／`MAX_FRAGMENTS:39`／`FRAGMENT_TIMEOUT:42`／`fragment_envelope_line:160`／`Reassembler:248`）＋`src/protocol.rs`（`DeviceMsg::AipFrag:131`／`HostMsg::AipFrag:171`／`accept_fragment:635`／`expire_fragments:699`／`supports_fragmentation:719`／`send_aip:736`）；模擬器對端 `scripts/esp32-serial-sim.py:140`（`--no-frag`）；契約 `docs/aip/device-profile.md` §6.3 |

行號對應 `9799b1e`；行號會漂，冒號前的符號名才是錨點。

### 3.3 `reason: "recovery"`：舊接收端把它當成沒有 reason 的 snapshot

| 欄位 | 內容 |
|---|---|
| 為什麼存在 | `recovery` 是接收端澄清新增的 reason **值**（訊息形狀、欄位、版本號一律不變）。只認得舊值的接收端會走「同 epoch、revision 較舊」那一格，也就是 `ignore-stale`／rollback 忽略——行為與今天完全相同，不會更糟，但也**收不到** host 真的從較舊快照還原這件事 |
| 適用版本 | 送出端自 v0.7.0；接收端 Rust／TypeScript／Swift 已接線 |
| 移除前需要的證據 | 這是「新增值」而非 deprecation，沒有移除計畫。要登記的是：任何新的接收端實作若沒有規則 6，就會靜默退化成 rollback 忽略——三端一致由跨語言 fixture 保證（`crates/interaction-aip/tests/fixtures/manifest.json` 的 `receiveDecisions`） |
| 資料遷移 | 無 |
| 回退方式 | host 停止送出 `reason: "recovery"`，退回無 reason 的 snapshot |
| 下一檢查里程碑 | 下一個新的接收端實作 |
| owner | `crates/interaction-session/src/types.rs`（`REASON_RECOVERY`）＋`src/receive.rs` 規則 6；契約 `docs/aip/character-session.md` §7.2 |

## 4. Feature flag 與可觀測欄位

### 4.1 `INTERACT_AI_CHARACTER_SESSION`（env，預設 `1`）

| 欄位 | 內容 |
|---|---|
| 為什麼存在 | Session Host 的**回退路徑**：`0` 時 Runtime 不啟動 Session Host，`/v1/character-session/*` 回 `503 session-disabled`，iPhone 的 `aip` frame 回 `error{unsupported-capability}`，其餘行為與 v0.5.1 相同 |
| 適用版本 | v0.6.0 起 |
| 移除前需要的證據 | Session Host 在真機（iPhone）與真板（ESP32）上各有一次成功的閉環證據——**兩者目前都是零**。旗標是唯一一條「出事了先關掉」的路，在取得真環境證據前不得移除 |
| 資料遷移 | 無。注意 `session-disabled` 是 `ErrorCode::KNOWN` 的第 19 個值（`compatibility.md` §4.1），移除旗標不等於可以移除錯誤碼 |
| 回退方式 | 旗標本身就是回退方式 |
| 下一檢查里程碑 | 第一筆 iPhone 真機 AIP 閉環證據 |
| owner | `crates/interaction-runtime/src/character_session.rs`（`character_session_enabled_from_env`）；契約 `docs/aip/architecture-boundaries.md` §5 |

### 4.2 桌面進階診斷計數的名稱更換

| 欄位 | 內容 |
|---|---|
| 為什麼存在 | 決策表落地時，桌面 reducer 的計數改名並拆細：`ignoredRollback` → `ignoredStale`、`hostRegressed` → `recovered`、`invalid` → `rejectedInvalid`，新增 `rejectedIdentity`／`staleConnection`。這些鍵直接以 `alignment.<key>` 顯示在**進階模式**的診斷區塊（一般模式一個數字都不顯示），所以任何盯著舊鍵名的外部筆記／截圖會對不上 |
| 適用版本 | 舊名到 v0.6.x；新名自 v0.7.0 |
| 移除前需要的證據 | 舊名已經移除（不是並存），這一列登記的是**沒有相容層**這個事實：`SessionCounters` 是 TypeScript 型別，改名由 `pnpm typecheck` 擋，不會有靜默的舊鍵 |
| 資料遷移 | 無（計數只存在於記憶體，不持久化、不上 wire） |
| 回退方式 | 改名回去；沒有外部消費者需要同時支援兩組名字 |
| 下一檢查里程碑 | 不需要（登記用，供讀舊截圖的人對照） |
| owner | `apps/interaction-desktop/src/aip/sessionClient.ts`（`SessionCounters`）；呈現在 `src/components/CharacterSyncCard.tsx` 的 `alignment.*` |

## 5. 加一條新的相容路徑時

1. 先問：能不能不加？（新增選填欄位／新 name／新 capability 鍵通常可以，見 `compatibility.md` §2）
2. 一定要加，就在本表新增一列，七個欄位全部填滿——「移除前需要的證據」欄空著等於這條路徑沒有退場計畫。
3. 如果它是 Rust API，同時加 `#[deprecated(since = …, note = …)]`；如果它是 wire 值，同時更新
   `compatibility.md` §1 矩陣。
4. 有資料要遷移的，遷移前先備份原檔，並在測試裡放一份**真實舊版本寫出來的** fixture
   （範例：`crates/interaction-runtime/tests/fixtures/character-session/`）。
