# Renderer／Adapter 邊界：收什麼、負責什麼、不負責什麼

> 證據等級用字：規則若有對應測試才寫測試名；沒有的一律標「契約文字，未見專屬測試」。
> 這份文件把 AIP `docs/aip/README.md`／`docs/aip/character-session.md` 與既有 CPP
> `docs/character-protocol/README.md` 兩份契約里對 Renderer／Adapter 的邊界說明收斂成一份，
> 給要寫或審 renderer adapter 的人看；不重新定義任何語意，只做導覽與收斂。

## 1. Renderer 收到什麼

| 訊息 | 協定 | 內容 |
|---|---|---|
| Semantic state（snapshot／patch） | AIP `state` | `mood`／`activity`／`attention`／`truth`／`lastInteraction`／`members`／`reducedMotion`（`docs/aip/character-session.md` §3） |
| Behavior Intent | AIP `command{name:"character.behavior.request"}` 或 CPP `intent{envelope}`（桌面 in-process 走投影後的 CPP 形狀） | `intent`／`intensity`／`interruptible`／`origin`／`hints`／`durationHint`／`priority`／`truthState`（CPP 補上；AIP 本身不帶 truthState） |
| Preferences（reconfigure） | CPP `hello`／`reconfigure`（既有，未變） | `reducedMotion`、角色專屬 `preferencesSchema` 白名單子集（`docs/character-protocol/README.md` §2.1） |
| 協商結果 | AIP `capability`（negotiated）／CPP `negotiated` | 每個 intent 的 `resolution`（`exact`／`substituted`／`unsupported`）、`acceptedChannels`／`ignoredChannels` |

Renderer **看不到**：原始感測資料、其他成員的身分（除了 diagnostics／members 列表裡的
`party`／`presence`）、consent grant 的內容（只看得到 `consentGrantId` 這個參照，1.0 目前
member 不會收到帶 grant 的訊息，見 `docs/aip/transport-bindings.md` §7）、Runtime token、
agent token。

## 2. Renderer／Adapter 負責什麼

1. **把 intent 轉成自己的呈現**（rig／參數／動畫／音效／DOM／燈光）——CPP §0 邊界圖的
   「Character Adapter」那一層；AIP 層的 `CppRendererAdapter`
   （`crates/interaction-session/src/cpp.rs::behavior_to_cpp`）只做語意投影
   （`intent`／`variant`／`parameters`／`priority`），**不**碰任何 rig 細節。
2. **誠實回報降級**：`resolution` 只能比協商結果更差，不能升級成 `exact`
   （CPP §7；`docs/character-protocol/README.md`）。
3. **不支援的 intent 只回 `unsupported`／不回**，不得回 `observed`／`completed`
   （`docs/aip/character-session.md` §5：「本地降級（idle／文字），回
   `result{status: rejected, code: unsupported-capability}` 或不回；不得回 `observed`」）。
4. **送受限的 input event**（CPP §6：正規化座標、頻率上限、宣告即契約）。
5. **crash／斷線時把 pending 標 `uncertain`**，不得補成 `completed`（CPP §7）。

## 3. Renderer／Adapter 不負責什麼（沒有權限主權）

依 CPP §0 不變量 1-7（`docs/character-protocol/README.md`）與 AIP 對應收斂：

| 不負責 | 為什麼 | 誰負責 |
|---|---|---|
| 改權威情緒／活動／注意力 | `SemanticState` 欄位私有，只有 `CharacterSession::apply` 能改（`docs/aip/character-session.md` §2：「Renderer／Device port 沒有任何 setter」） | Character Session Host（Director） |
| 判斷 consent | AIP 層沒有把 grant 判定邏輯交給任何 member（`ConsentVerifier` port 定義在 `crates/interaction-session/src/ports.rs:42-45` 但**目前沒有任何呼叫端使用它**——本次以 `rg -n "consent" crates/interaction-session/src` 核實零命中，是「這條路目前走不到」，不是「已驗過安全」） | Runtime Consent Service（既有，`interaction-policy`，本文件未盤點其原始碼） |
| 決定 `verified` | `crates/interaction-session/src/session.rs::gate`（:690-693）拒絕任何 member 送來的 `status:"verified"` | Runtime 人類驗證路徑（`verify_agent_session`） |
| 覆寫共享狀態 | 同上，`SemanticState` 沒有 setter；廣播出去的 `state` patch 由 host 端 `commit_and_patch`（`session.rs`:1067）產生 | Character Session Host |
| 偽造 human verification | CPP §6：「Adapter 不能偽造 human verification（沒有任何 event kind 能表達它）」 | — |
| 解除 Emergency Stop | CPP §0 不變量 7；外部 adapter 沒有這個路由 | Runtime |

## 4. 兩條路：桌面 in-process adapter 與 iPhone renderer

| | 桌面 in-process | iPhone |
|---|---|---|
| 協定 | TS `CharacterAdapter` 介面（既有 CPP）＋ AIP `state`／`command` 經 `character_session.rs` 廣播 | AIP `{"type":"aip","envelope":{…}}` frame（`docs/aip/transport-bindings.md` §1） |
| 註冊 | `character/adapterRegistry.ts::registerBuiltinAdapter(id, factory, meta)` | `capability` envelope（`role:"remote-renderer"`） |
| 身分 | `human-surface:desktop`（`docs/aip/transport-bindings.md` §0：不是 `renderer:desktop`） | `device:<deviceId>`（配對出來的） |
| Behavior Intent 投影 | CPP `intent{envelope}`（既有路徑；`react-happily-to-touch`→CPP `play`、`settle`→CPP `rest`、`idle`→CPP `idle`；`celebrate` 不投影，避免與既有 `verified-success` 雙播——`crates/interaction-session/src/cpp.rs`） | AIP `command{name:"character.behavior.request"}` 直送（iPhone 沒有既有真相投影，所以 `celebrate` 直接送給它） |
| 生命週期 | CPP 14 態 `AdapterLifecycleState`（既有） | AIP profile：`join`／`presence`／`resume` |

## 5. Contract test 檢查清單（`src/test/adapter-contract.test.ts`）

`describe("builtin adapter registry", …)`（`apps/interaction-desktop/src/test/adapter-contract.test.ts`:174）
對 shu-rig／sprite／text／shape 四個內建 adapter 跑同一套斷言（it 名稱與行號）：

| 檢查 | it 名稱 |
|---|---|
| 白名單不依賴載入順序 | `宣告的 id 都有工廠，白名單不依賴載入順序`（:175） |
| 未註冊 entrypoint 誠實失敗、不回顯輸入 | `未註冊的 entrypoint 誠實失敗，錯誤訊息不回顯輸入`（:190） |
| 四個 adapter 都建得出來，meta 一致 | `createBuiltinAdapter 建得出四個 adapter，meta 與 registry 一致`（:195） |
| Lifecycle／能力宣告 | `生命週期順序：註冊後 ready、negotiate 提供 manifest 宣告的能力`（:218） |
| Unsupported 永不回 completed | `unsupported resolution：回 unsupported，永遠不回 completed`（:252） |
| Cancel 冪等 | `cancel 冪等：重複 cancel 不再產生回執`（:263） |
| Timeout 由 tick 推進 | `timeout：durationHint 由 tick(now) 推進，時間沒到不會提前 completed`（:279） |
| Dispose 後不再送輸入／回執 | `dispose 後：perform 只回 failed，不再送輸入事件`（:294） |
| 重複訂閱不重複送 | `重複訂閱：兩個 callback 各收一次，退訂只退自己那一份`（:308） |
| Dispose 後資源歸零（timer／rAF／DOM listener） | `資源清理：dispose 後 timer／rAF／DOM listener 都回到原本水位`（:333） |

這套 contract test 是本文件對「Renderer／Adapter 不負責什麼」最直接的執行期驗證來源：
unsupported 不回 completed、dispose 後不再回執、重複訂閱不重複送，全部有測試名可查，
不是只有契約文字。證據等級：unit（vitest，本次以 `git show HEAD:` 核對測試名與其所在行號存在，
未重新執行 `pnpm test`）。

## 6. Unsupported 的誠實降級

依 CPP §3.4 解析演算法（`docs/character-protocol/README.md`）：manifest 沒宣告的 intent
一律 `unsupported`（非安全 intent）或落 `system.text`（安全 intent，`system.text` 是 Runtime 提供、
永遠可用的最後退路）。`ref-shape` 是這條規則的活範例（`docs/aip/reference-character.md` §4）：
安全 intent 不宣告，一律落 `system.text`，「adapter 沒有否決權，也不會假裝演過」。
