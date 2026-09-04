# Character Presentation Protocol（CPP）v1.0 — Canonical Contract

> 這是**唯一**的角色呈現協定規格。Rust（`crates/interaction-character`）是權威實作與 JSON Schema 來源
> （`schemas/character-protocol.schema.json`）；TypeScript（`apps/interaction-desktop/src/character/`）鏡射同一份型別。
> 任何引擎（參數化 2D rig、sprite、Live2D、Spine、Rive、3D、影片、DOM、遠端裝置、純聲音／燈光）都透過這一份契約接上
> Runtime；小樞只是附帶的第一個完整 Reference Adapter，不是 Runtime 唯一支援的角色規格。

## 0. 邊界

```
Adaptive Interaction Runtime（真相、安全、同意、預算）
        │  語意化 Character Intent（含 truthState、priority 下限）
        ▼
Character Presentation Gateway（能力協商、排程、去重、過期、降級、回執、世代）
        │
        ▼
Character Adapter（把 intent 轉成自己的 rig／參數／動畫／音效／燈光）
        │
        ▼
任意 Renderer／Engine／Device

Renderer／角色互動 ──受限 Interaction Event──▶ Gateway（正規化、節流、隱私）──▶ Runtime（receptor／policy）
```

不變量：

1. **呈現層沒有權限主權**。Adapter 只能收到 Runtime 授權後的 intent，或送出受限的 input event。
2. **truthState 只由 Runtime 決定**。Adapter／Character Pack／第三方程式不能把 `claimed` 改成 `verified`，也不能自己產生 `verified`。
3. **不支援就誠實降級**（`substituted`／`reduced`／`unsupported`），不得假裝 `exact`；沒有任何呈現能力時，安全訊息回退到 Runtime 的 `system.text`（文字／通知），不得遺失。
4. **accepted ≠ started ≠ completed**；`completed` 只代表「呈現演完了」，永遠不等於外部工作 `verified`。
5. Emergency／offline／blocked／unknown／waiting-consent 的 priority 由 Runtime 固定下限，可搶占任何非安全演出；custom channel 不能影響安全搶占。
6. Adapter crash／斷線時，進行中的 command 一律 `uncertain`，不得補成 `completed`；舊世代（generation）的回執不得污染新連線。
7. 外部 adapter 永遠拿不到 human token、agent token、policy 修改、consent 授予、解除 Emergency Stop、verified 判定。

## 1. 名詞

| 名詞 | 意義 |
|---|---|
| Character Definition | 一份 manifest（`characterId` 穩定身分＋能力宣告＋資產） |
| Character Adapter | 能執行 manifest 的程式（in-process TS、外部程序、遠端裝置） |
| Character Instance | Adapter 的一次實例（`characterInstanceId`；有自己的位置、外觀、可見性、世代） |
| Character Role | `primary-companion`／`familiar`／`worker`／`observer`／`notification-only` |
| Gateway | Runtime 內（Rust）與桌面視窗內（TS）各有一個；語意相同 |

## 2. Manifest（`CharacterManifest`）

```jsonc
{
  "schemaVersion": "1.0",                       // 協定 schema 版本（major.minor）
  "characterId": "shu-maid",                    // ^[a-z0-9][a-z0-9._-]{0,63}$；穩定身分，不等於顯示名稱
  "displayName": { "zh-TW": "小樞", "en": "Shu" },   // LocalizedText；每個值 ≤ 48 字
  "author": "adaptive-interaction",             // ≤ 120 字
  "description": { "zh-TW": "…" },              // 每個值 ≤ 400 字
  "version": "3.0.0",                           // 角色版本（semver 字串）
  "adapterKind": "in-process",                  // in-process | web | external-process | remote-device
  "entrypoint": { "kind": "builtin", "id": "shu-rig" },
  //   builtin{id}            in-process 內建 adapter（白名單：shu-rig / sprite / text）
  //   module{path}           web：相對路徑模組（**匯入時不會執行**，只在使用者啟用後由 host 載入）
  //   process{command[]}     external-process（**永不自動啟動**；需明確安裝與授權）
  //   url{url}               remote-device（ws://127.0.0.1 或已配對裝置；**永不自動連線**）
  "assets": [ { "id": "sheet", "path": "sheet.png", "mediaType": "image/png", "bytes": 12345, "sha256": "…" } ],
  "capabilities":      { "<capabilityId>": CapabilityDecl },   // 見 §3
  "inputCapabilities": { "<capabilityId>": CapabilityDecl },
  "channels": ["transform", "pose", "expression", "com.example.character.wings"],
  "states":  ["idle", "working"],               // 資訊性：adapter 自己的狀態 id
  "intents": ["idle", "notice", "work"],        // 原生支援的 canonical intent（§4）
  "variants": [ { "id": "maid-classic", "displayName": { "zh-TW": "經典" } } ],
  "locales": ["zh-TW", "en"],
  "pronouns": { "zh-TW": "她", "en": "she" },   // 可省略；省略時 UI 用中立文案（「角色」／they）
  "preferencesSchema": { "type": "object", "properties": { … } },   // §2.2 白名單子集
  "securityRequirements": { "network": false, "executable": false, "fileAccess": "none",
                            "audioOutput": true, "microphone": false, "camera": false },
  "resourceLimits": { "maxAssetBytes": 8388608, "maxConcurrentCommands": 4, "maxQueue": 32, "maxFps": 60 },
  "fallbacks": { "capabilities": { "visual.expression": ["visual.pose", "visual.textBubble"] },
                 "intents": { "play": "notice", "sleep": "rest" } },
  "compatibility": { "protocol": "1.x", "runtime": ">=0.5.0" }
}
```

### 2.1 驗證規則（匯入／載入時，Rust 與 TS 都要做）

- 檔案大小 ≤ 256 KB；`assets` ≤ 64 項；單一資產 `bytes` ≤ `resourceLimits.maxAssetBytes`（上限 32 MB）。
- `schemaVersion` major 必須等於 1；minor 大於實作者時允許並記錄 `newerMinor: true`（未知欄位保留、不崩潰）。
- `characterId` 正則；`displayName` 至少一個 locale；所有 LocalizedText 值長度上限如上。
- `assets[].path`：相對路徑、不得含 `..`、不得以 `/` 或磁碟代號開頭、不得含 `\\`、不得為 URL；**MIME／副檔名不可作唯一信任依據**（載入時以 magic bytes 核對 png/jpg/gif/webp/svg/json/mp3/wav/ogg/webm）。
- `entrypoint`：`process`／`url`／`module` 只允許記錄，**匯入不執行、不連線、不下載**；`builtin.id` 必須在 host 白名單內。
  白名單**由 host 注入**：協定核心（`interaction-character`）不認識任何具名 adapter，
  `ValidationLimits::default().builtin_whitelist` 是**空的**（＝這個 host 不提供任何 builtin 角色，所有
  builtin entrypoint 都會被拒）。桌面 host 的來源是
  `interaction_runtime::character::character_host_registry()`（Rust，同時供 Tauri 匯入路徑使用）與
  `apps/interaction-desktop/src/character/adapterRegistry.ts` 的 `builtinEntrypointIds()`（TS）。
  兩邊目前都是 `shu-rig`／`sprite`／`text`／`shape`；`shu-rig` 的 id 由
  `crates/interaction-character-shu` 提供，核心不再有這個字串。
- `capabilities` 鍵：canonical id（§3）或 namespaced custom（`^[a-z][a-z0-9]*(\.[a-z][a-z0-9]*){2,}$`，至少三段，例如 `com.example.character.wings`）；未知 canonical 前綴（`visual.`／`audio.`／`haptic.`／`light.`／`input.`）但未收錄的 id 視為 custom 並標 `unknown: true`。
- `preferencesSchema`：只接受 `type: object`，`properties` ≤ 32 個，每個屬性只允許 `boolean`／`number`（含 `minimum`／`maximum`）／`string`（`maxLength` ≤ 200、可有 `enum` ≤ 16）／`integer`；不允許 `$ref`、`pattern`、巢狀物件、陣列。
- 錯誤訊息不得回顯超過 200 字的輸入內容、不得包含絕對路徑。
- 未簽章、`securityRequirements.executable=true`／`network=true`／`adapterKind ≠ in-process` 的 manifest 在 UI 必須標示「第三方／外部／需要網路／有可執行程式」。

### 2.2 Migration（既有 Character Pack → Manifest）

| 舊格式 | 對應 |
|---|---|
| `character-pack` 1.0／1.1（sprite sheet） | `adapterKind: in-process`、`entrypoint: {builtin: "sprite"}`、`assets: [sheet]`、`capabilities: visual.presence/visual.expression(variants = animations)`、`inputCapabilities: input.click/input.drag/input.drop/input.text/input.fileDrop`、`fallbacks` 由舊 FALLBACKS 鏈轉入 |
| `character-rig` 2.0（`palette` only） | `entrypoint: {builtin: "shu-rig"}`、`variants` = 三個 palette、完整能力集（§12） |
| persona-pack／story-pack | 不變（純資料、安全語句固定）；manifest 以 `preferences.persona`／`preferences.story` 引用 id |
| `DesktopPrefs.companionPack`（8 個 shu-* id） | 視為 `characterId`；`shu-standard`→sprite adapter；`shu-maid*`→shu-rig adapter |

舊 id 一律可用；匯入舊 pack JSON 時自動產生 manifest，不改寫使用者設定。

**Migrator registry（v0.6.0）**：遷移由 host 註冊的 `PackMigrator` 依 (`kind`, `schemaVersion`) 分派
（`interaction_character::MigrationRegistry`，有界：≤ 32 個 migrator、每個 ≤ 8 個 schemaVersion，
同一組 (kind, version) 不得註冊兩次）。核心只內建通用 sprite（`character-pack` 1.0／1.1，
`SpritePackMigrator`）；`character-rig` 2.0 由 `interaction-character-shu` 的 `RigPackMigrator` 提供，
桌面 host 在 `character_host_registry()` 裡註冊。入口是
`migrate_pack_to_manifest(json, &registry)`；舊的 `migrate_legacy_pack` 已 `#[deprecated]`，
只剩 sprite 一條路（核心不能依賴任何角色 crate）。沒有 migrator 的格式一律誠實拒絕
（`ManifestErrorCode::Legacy`），不猜、不執行。TS 端的 `migratePackToManifest` 語意不變。

## 3. 能力（Capability）與協商

### 3.1 Canonical capability ids

```
visual.presence      visual.pose          visual.expression    visual.gaze
visual.locomotion    visual.overlay       visual.particles     visual.prop
visual.textBubble    audio.speech         audio.effect         haptic.cue
light.cue            input.click          input.hover          input.drag
input.drop           input.pointerProximity   input.text       input.fileDrop
multiCharacter       scene                rollCall             gameplay.toys
gameplay.autonomy    system.text   （由 Runtime 提供、永遠可用的最後退路：文字／通知）
```

### 3.2 `CapabilityDecl`

```jsonc
{ "supported": true, "version": "1", "variants": ["idle", "notice"], "maxConcurrent": 1,
  "interruptible": true, "resumable": true, "durationRange": { "minMs": 200, "maxMs": 60000 },
  "parameterSchema": { … 同 §2.1 preferencesSchema 規則 … }, "qualityLevel": "full" | "reduced" | "minimal",
  "reducedMotionBehavior": "static" | "reduced" | "unchanged" | "disabled",
  "requiresForeground": false, "requiresAudio": false }
```

### 3.3 握手

1. Runtime／Gateway → Adapter：`hello`
   `{ type:"hello", protocolVersion:"1.0", runtimeVersion, characterInstanceId, role, locale, reducedMotion,
      requires:[…intent ids the runtime will send…], limits:{maxMessageBytes, maxMessagesPerSecond, maxPending} }`
   - `reducedMotion` 只有一個主人：可信 host（桌面視窗）在 `POST /v1/character/hello` 的 body 帶
     `reducedMotion`（`prefers-reduced-motion`／使用者偏好），Gateway 以它協商並記在該 instance 上；
     adapter 不能自己宣告。使用者中途改設定 → 視窗重送 hello（generation+1）重新協商。
     外部 WebSocket adapter 目前沒有自己的來源，收到的是該 instance 的現值（預設 false）。
2. Adapter → Gateway：`negotiate`
   `{ type:"negotiate", protocolVersion, characterId, manifestVersion, capabilities, inputCapabilities, channels,
      intents, variants, generation }`
   - `protocolVersion` major 不同 → Gateway 回 `error{code:"protocol-version"}` 並拒絕（不猜）。
3. Gateway 計算 `NegotiatedCapabilities` 並回 `negotiated`
   `{ type:"negotiated", characterInstanceId, generation, reducedMotion,
      resolutions: { "<intent>": { resolution, via:"<capabilityId>", variant? } },
      acceptedChannels:[…], ignoredChannels:[…], capabilities:{…最終有效宣告…} }`

### 3.4 解析演算法（deterministic）

對 Runtime 需要表達的每個 intent（§4.1 全部 20 個）：

1. 若 `intents` 含該 intent 且對應能力 `supported` → `exact`。
2. 否則依 `fallbacks.intents[intent]` 換成另一個 intent（一次），能達成者 → `substituted`（`via` 記錄實際 intent）。
   **安全 intent（`request-consent`／`blocked`／`unknown`／`failed`／`cancelled`／`offline`／`emergency`）只能換成另一個安全 intent**：
   來源是安全 intent 而目標不是，manifest 驗證直接回錯誤（Rust `ManifestErrorCode::Fallbacks`／TS `c.err`），協商時也跳過本步驟
   （落到步驟 3／5，最差誠實落 `system.text`）。舊 Character Pack 遷移不再產生 `emergency→sleep`、`blocked→sleep`、`ask→notice`
   這類映射（v0.5.1，對抗審查 character-protocol-036）。
3. 否則依 `fallbacks.capabilities[cap]` 鏈往下找第一個 `supported` 的能力 → `substituted`。
4. `reducedMotion=true` 時，若所用能力 `reducedMotionBehavior ∈ {static, reduced}` → `reduced`；`disabled` → 繼續往下一個 fallback。
5. 什麼都沒有 → 安全 intent（§4.3 有 floor 者）一律解析為 `via:"system.text"`、`resolution:"substituted"`；非安全 intent → `unsupported`。
6. 執行期沒演成 → Gateway 對安全 intent 自動改走 `system.text`。**只有 `completed` 算演到使用者眼前**：安全 intent 的任何其他終態（`failed`／`unsupported`／`cancelled`／`expired`／`uncertain`，不論來自 adapter 回執、逾時／watchdog 掃描、斷線或重新協商）都會用衍生 messageId（`<原 id>/system-text`）補一則 `system.text`。呈現層對安全訊息沒有否決權：adapter 不能靠挑一個合法終態把 emergency／blocked／request-consent／offline 吞掉。

`unknown` custom channel：namespaced 者進 `acceptedChannels` 但標 `nonSafety`，非 namespaced 者進 `ignoredChannels`；兩者都不能影響 priority、truthState 或搶占。

## 4. Character Intent

### 4.1 詞彙（20）

```
idle  notice  acknowledge  think  work  wait  ask  request-consent  blocked  unknown
claim-completed  verified-success  failed  cancelled  offline  emergency  greet  play  rest  sleep
```

### 4.2 `truthState`（只由 Runtime 設定）

```
none  queued  working  waiting-input  waiting-consent  blocked  claimed  verified
failed  timed-out  expired  unknown  cancelled  emergency  offline
```

`verified` 只能來自人類驗證路徑（`verify_agent_session`／`action.observed`）；Gateway 拒絕任何來源不是 Runtime 的 `verified`。

### 4.3 Priority 下限（Runtime 固定；`priority = max(requested, floor)`）

| intent | floor | | intent | floor |
|---|---:|---|---|---:|
| emergency | 100 | | verified-success | 70 |
| offline | 95 | | claim-completed | 65 |
| blocked | 90 | | wait／ask | 60 |
| failed | 85 | | cancelled | 55 |
| request-consent | 80 | | 其他（idle／notice／acknowledge／think／work／greet／play／rest／sleep） | ≤ 50（AI 請求上限 50） |
| unknown | 75 | | | |

### 4.4 Envelope

```jsonc
{ "protocolVersion": "1.0", "messageId": "uuid", "characterInstanceId": "…", "correlationId": "action/session id",
  "timestamp": "RFC3339", "intent": "work", "truthState": "working", "priority": 40,
  "interruptPolicy": "preempt" | "queue" | "drop-if-busy" | "merge",
  "resumePolicy": "resume-previous" | "return-idle" | "none",
  "durationHint": { "ms": 4000, "loop": true },
  "parameters": { … ≤ 4 KB、依 capability parameterSchema 驗證、字串 ≤ 200 字 … },
  "presentationHints": { "tone": "neutral", "message": "≤200", "variant": "curious", "channels": { … } },
  "privacyClass": "public" | "internal" | "personal" | "intimate",
  "expiresAt": "RFC3339" }
```

規則：過期不播（`expired` 回執）；重複 `messageId`（環 256）去重並回 `accepted{duplicate:true}`；`presentationHints` 只是建議；`correlationId` 串起 Agent 工作、硬體事件、receipt 與演出；AI 不能直接構造 envelope（AI 只能透過 `companion.state.present` 等受 policy 管制的 actuator 請求，Runtime 轉成 envelope 並強制 floor ≤ 50、`truthState: none`）。

AI 的 presentation 命令會用同一個 `messageId` 廣播給所有已連線角色，但**只有桌面本尊的回執會推進那筆 actuator receipt**（沒有桌面連線時是第一個目標）；其他 instance 的回執只進 audit／`character.receipt` 事件。否則只有 adapter token 的外部角色搶先回一則 `completed`／`failed`，就替桌面決定了 AI 看得到的結果——而 §8.2 明訂 adapter token 不能呼叫 actuator。

## 5. Semantic channels

`transform locomotion pose expression gaze speech bubble audio prop overlay particle scene` ＋ namespaced custom。
Adapter 自己把 channel 映射到身體部位／Live2D 參數／Spine 動畫／DOM class／LED；Runtime 不引用任何部位名稱。
Mixer 規則：每個 channel 一個 owner；`priority` 高者可搶占；`interruptible=false` 的演出只能被 floor ≥ 75 的 intent 搶占；
被搶占者回執 `cancelled{reason:"preempted"}`；`resumePolicy=resume-previous` 時安全演出結束後恢復。Reduced Motion 由協商決定。

## 6. Input events

```
character.clicked  character.double-clicked  character.hover-entered  character.hover-left
character.drag-started  character.dragged  character.dropped  character.text-submitted
character.file-dropped  character.toy-thrown  character.action-requested  character.dismissed
character.visibility-changed
```

Envelope：`{ protocolVersion, eventId, characterInstanceId, generation, timestamp, kind, payload, privacyClass }`。

正規化與限制（Gateway 強制）：
- 高頻：`hover-*` ≤ 4/s、`dragged` 合併為 ≤ 10/s 且只帶量化座標（8 px 網格，視窗相對）、`pointerProximity` ≤ 1/30 s；佇列上限 64，滿了丟最舊的非安全事件。
- **不保存原始游標軌跡、不送 AI**；payload 不含絕對螢幕座標。
- **宣告即契約**（v0.5.1，character-protocol-040）：`clicked`／`double-clicked`／`action-requested` 需要 `input.click`、`hover-entered` 需要 `input.hover`、
  `drag-started`／`dragged` 需要 `input.drag`、`dropped` 需要 `input.drop`、`text-submitted` 需要 `input.text`、`file-dropped` 需要 `input.fileDrop`。
  manifest（協商後為 `negotiated.inputCapabilities`）沒宣告的種類在進佇列前就被丟棄（`{decision:"dropped", reason:"capability-not-declared", requires}`）、
  留稽核 `character.input-capability-not-declared`，不產生任何 `companion.*` 觀察；連接頁「可以接收：…」由同一份 manifest 產生，不會說一套做一套。
  `hover-left`／`toy-thrown`／`dismissed`／`visibility-changed` 只留稽核，不設閘。
- `text-submitted` ≤ 2000 字；`file-dropped` 只帶 `{name, mediaType, bytes, readableScope, grantId, expiresAt}`，grant 短效（≤ 10 分鐘）、只授權該檔案、可撤銷；不授權整個檔案系統。
- `action-requested{action}` 只是請求：**可信 host 表面（桌面視窗，`/v1/character/hello` 註冊的 instance）**的事件才會被 Gateway 轉成 `companion.quick-action` receptor observation，仍經 Runtime policy／consent。
- **外部 adapter（`/v1/character/ws`＋adapter token，origin `external`）的輸入事件不會變成任何 `companion.*` 觀察**：只留稽核（`character.input-not-observed`），HTTP 回 `{decision:"audit-only", reason:"external-adapter-input-not-observed"}`。
  **v0.5.1 決定**：維持這個安全拒絕。要讓外部 adapter 真的回報使用者互動，需要專屬的 receptor 命名空間（例如 `character.external.click`／`hover`／`drag`／`drop`／`touch`）配上 manifest 宣告、人類同意、surface／instance 綁定（`ConsentScope` 目前沒有 instance 維度）、速率限制、payload 正規化（不存原始軌跡）、origin 身分、稽核與撤銷，且 adapter 只能回報自己 surface 的互動、不得偽造 OS 全域輸入——這些牽涉 `interaction-core` 的 consent／observation 資料模型與 recipe 觸發語意，超出 patch version 範圍，列為下一個 minor version 的設計項目；本版沒有任何管道讓外部 adapter 的輸入落成觀察。理由：那些 receptor 是桌面表面的受器，寫進去會繞過「隱藏角色停用視窗內受器」的閘門、觸發只比對 receptor id 的 recipe，並用假的「使用者有回應」解除主動對話的不追問守則——呈現層沒有權限主權，也就不該有合成人類輸入的權力。
- 普通角色互動不啟動工作 Agent；角色輸入不能直接變成 OS／硬體／檔案系統操作；Adapter 不能偽造 human verification（沒有任何 event kind 能表達它）。
- 多角色：每個 event 都帶 `characterInstanceId`；Gateway 依 role 過濾（`observer`／`notification-only` 不送輸入）。

## 7. 生命週期與回執

Adapter 生命週期：`discovered → loading → validated → initializing → negotiating → ready → shown ⇄ hidden → suspended ⇄ resumed → reconfiguring → disposed`，另有 `crashed`／`reconnecting`。

Command 回執（`CommandReceipt`）：

```jsonc
{ "messageId": "…", "characterInstanceId": "…", "generation": 3,
  "status": "accepted" | "acknowledged" | "scheduled" | "started" | "completed" | "cancelled" | "expired" | "unsupported" | "failed" | "uncertain",
  "resolution": "exact" | "substituted" | "reduced" | "unsupported" | "failed",
  "detail": "≤200", "at": "RFC3339" }
```

- 合法順序：`accepted → (scheduled) → started → completed | cancelled | failed`；`accepted → expired | unsupported`；
  `acknowledged` 代表「收到但這個 adapter 不會回報 completion」，Gateway 之後把它記成 `uncertain`（不猜 completed）。
- `completed` 只代表呈現 adapter 完成演出；Runtime 端 receipt 的 verification 永遠是 `acknowledged-only`。
- `cancel` 冪等：重複 cancel 同一 messageId 回同一結果；對已終結的 command 回 `cancelled{alreadyTerminal:true}` 不報錯。
- crash／斷線／`goodbye`：Gateway 把所有 pending 標 `uncertain`、釋放 timer／audio／physics／rAF、`generation += 1`；舊 generation 的回執與事件一律丟棄（記 audit）。斷線當下**還在演的安全 intent** 會以衍生 messageId（`<原 id>/system-text`）改走 `system.text`——安全訊息不因 adapter 掛掉而遺失。
- 重新協商（`negotiate`）：pending 一律 `uncertain`，其中**還在演的安全 intent** 比照斷線補 `system.text`——adapter 不能靠重送 `negotiate` 讓安全訊息無聲消失。
- 回執的 `resolution` 只能比協商結果**更差**：adapter 可以誠實回報降級（`reduced`／`substituted`／`unsupported`／`failed`），Gateway 不接受升級成 `exact`（Reduced Motion 下也不會被改寫成 `exact`）。`status:"failed"` 的 resolution 一律 `failed`、`status:"unsupported"` 一律 `unsupported`：回執不會出現「狀態說沒演成、resolution 說 exact」這種自相矛盾。
- 外部 adapter：heartbeat 每 15 s、45 s 無訊息視為斷線、重連退避 1 s → 15 s（倍增）、每次重連重新 `hello`。
- 回執進入 Runtime 的 audit／event（`character.receipt`），但**不**改動任何工作 verification。

## 8. Wire messages（transport-neutral JSON）

```
runtime → adapter : hello | negotiated | intent{envelope} | cancel{messageId, reason} | heartbeat | error | goodbye
adapter → runtime : negotiate | receipt{receipt} | event{event} | lifecycle{state} | heartbeat | error | goodbye
```

限制：單則 ≤ 64 KB；每個 adapter ≤ 50 則/s（超過 → `error{code:"rate-limited"}` 並丟棄）；pending intents ≤ 64；outbound 佇列 ≤ 32（滿了丟**剛進來的**非安全訊息並留稽核；安全訊息不丟——最多 8 則在有界等待（5 s）內等空位，
等不到就結算 uncertain＋`system.text`；WS 寫入逾時 5 s 即斷線並把 pending 結算為 uncertain）。「淘汰隊首」需要自管佇列，v0.5.1 未做。

50 則/s 是**每個 instance 一份預算，所有入口共用**：WebSocket 訊息、`POST /v1/character/receipts`
（超量回 `{accepted:false, status:"rate-limited"}`）、`POST /v1/character/events`（超量回
`{decision:"dropped", reason:"rate-limited"}`），以及連 JSON 都解不開的畸形 frame——畸形訊息**先扣預算再處理**，
不能靠丟垃圾繞過限制。畸形訊息的稽核也有界：同一個 instance 每 5 秒最多留下一列
`character.wire-rejected`，其間被壓下的次數記在下一列的 `suppressed`。

### 8.1 Transports

| Transport | 用途 | 狀態 |
|---|---|---|
| In-process（TS `CharacterAdapter` 介面） | 桌面視窗內建 adapter（小樞 rig、sprite、text） | **已實作（implemented）** |
| Runtime ↔ 桌面視窗 | `character.intent` 事件（SSE／Tauri IPC）＋ `POST /v1/character/receipts`、`POST /v1/character/events`（human token；桌面視窗是可信 host） | 已實作 |
| WebSocket `GET /v1/character/ws?token=<adapter token>` | 外部程式／遊戲引擎／遠端顯示（loopback） | **已實作（implemented）**：reference fixture＋CLI E2E 閉環 |
| stdio JSON Lines | 本機子程序 | **規格已定、未實作（specified / not implemented）**：訊息同 §8、一行一則；host **不**自動啟動子程序、沒有 host 端 spawn、沒有 stdio fixture、CLI E2E 也不涵蓋（外部 fixture 走 WebSocket）。文件範例不是完成證據；若要實作，屬下一個 minor version 的範圍 |
| HTTP | 管理：`/v1/character/adapters`（註冊／撤銷）、`/v1/character/instances`、`/v1/character/manifest`、`/v1/character/intent`（人類手動測試非安全 intent；安全 intent 一律 403） | 已實作 |
| SSE | 只讀事件訂閱（`character.intent`／`character.receipt`／`character.instance`）；不適合雙向控制 | 已實作 |

新增 transport（host 端）：本版**沒有**可插拔的 `CharacterTransport` trait（`interaction-character` 是純函式 crate，
沒有 tokio、沒有 I/O，見 §1）。外部 transport 目前只有 WebSocket 一種具體實作：`crates/interaction-api/src/character_ws.rs`
的 `character_ws_loop` 驅動 `interaction_runtime::character::WsSession`（`rx: mpsc::Receiver<WireMessage>` 有界 32、
第一則必為 `hello`；`close: CancellationToken` 由撤銷／取代／heartbeat 逾時觸發），inbound 走
`Runtime::character_ws_message` → `WsStep`。要加 stdio 等新 transport，需在 `interaction-runtime`／`interaction-api`
層仿照這個迴圈實作（不是在 `interaction-character` 裡定義 trait），且只能沿用 §8 的 JSON 訊息，不得另外定義訊息語意。

### 8.2 Adapter token

`POST /v1/character/adapters {displayName, manifest}`（human token）→ `{adapterId, token}`；token **只能**用於 `GET /v1/character/ws` 與自己 instance 的 `POST /v1/character/receipts`／`POST /v1/character/events`；不能讀任何人類路由（含 `/v1/status`、`/v1/events`）、不能呼叫 actuator、不能改 policy／consent／estop、不能 verify。token 以 sha256 儲存；`DELETE /v1/character/adapters/{id}` 撤銷並立即斷線（goodbye＋close）。外部 adapter 註冊時以 role `familiar` 加入（與桌面主角色分屬不同安全去重類別，兩者都會收到安全 intent）。

## 9. 安全模型（第三方角色）

| 威脅 | 防線 |
|---|---|
| 路徑穿越 | §2.1 路徑規則；host 只從角色資料夾內讀 |
| 任意 script／binary 自動執行、遠端資產靜默下載、任意網路連線 | `entrypoint` 只記錄；in-process 只允許 builtin 白名單；外部程序需人類明確安裝＋授權；資產只從本機資料夾載入 |
| 取得 Runtime token／agent token | adapter token 分權（§8.2）；WebSocket 不接受 human／agent token |
| 修改 policy／consent／memory／verification、解除 Emergency Stop | adapter 沒有這些路由；Gateway 不接受 `truthState`／`verified` 來自 adapter |
| 隱藏感測指示、改寫安全固定文字、偽造 verified-success | 感測與 estop 指示由可信 host（tray／host overlay 視窗）與 `system.text` 保證；安全語句固定在 Runtime／host |
| 無界 queue、巨大資產、動畫炸彈、記憶體耗盡 | §2.1 大小上限、§8 頻率／佇列上限、`durationRange` 上限 60 s、`maxConcurrentCommands` |
| 游標／檔案／對話／個資外傳 | §6 不含原始軌跡；file-drop 只 metadata＋短效 grant；`privacyClass` 標記；adapter 看不到對話內容（只看 intent） |
| Adapter crash 拖垮 Runtime | 外部 adapter 是獨立連線／程序；crash → `uncertain` ＋ fallback，Runtime 不受影響 |

分級：**內建純資料角色**（sprite／rig manifest、無可執行內容）vs **可執行 adapter**（external-process／remote-device）。後者必須顯示來源、作者、版本、能力、網路需求、資料範圍，並提供停用／撤銷／移除／fallback。

## 10. 版本相容政策

- `protocolVersion` 採 `major.minor`。同 major 相容：未知欄位保留、未知 intent 回 `unsupported`、未知 event kind 由 Gateway 丟棄（記 audit）、未知 capability 視為 custom。
- major 不同：握手拒絕。
- Runtime 保證：安全 intent 名稱、truthState 名稱、priority floor 在 1.x 內不改變；只會新增。
- JSON Schema：`schemas/character-protocol.schema.json` 由 Rust 產生（golden test），TS fixture 由 Rust 驗證器測試。

## 11. Runtime 如何產生 intent（truth projection）

| Runtime 事件 | intent | truthState |
|---|---|---|
| agent.session.state created／queued | wait | queued |
| fetched | think | working |
| working | work | working |
| waiting-input | ask | waiting-input |
| waiting-consent | request-consent | waiting-consent |
| claimed-completed | claim-completed | claimed |
| verified（human） | verified-success | verified |
| failed／timed-out | failed | failed／timed-out |
| unknown | unknown | unknown |
| cancelled／closed | cancelled／idle | cancelled／none |
| action.dispatched（非角色 actuator） | work | working |
| action.acknowledged | acknowledge | working |
| action.completed | claim-completed | claimed |
| action.observed | verified-success | verified |
| action.uncertain | unknown | unknown |
| action.failed | failed | failed |
| plan.blocked | blocked | blocked |
| emergency.stop／cleared | emergency／idle | emergency／none |
| proactive.paused／resumed | rest／idle | none |
| provider.state-changed available／paired | greet（hint device-online） | none |
| provider.state-changed disconnected／revoked | notice（hint device-offline） | none |
| receptor.observation | notice（hint listening） | none |
| companion.state.present（AI behaviorIntent） | rest／notice／think／work／acknowledge；`wait-attention`→**think**（variant `wait-attention`）、`look-at-confirmation`→**notice**（variant `look-at-confirmation`）——AI 永遠不能點播有 floor 的 intent（Rust `ai_safe_substitute`／TS `aiSafeSubstitute`） | none（priority ≤ 50） |

Runtime 端節流（避免佇列與 DB 無界）：`receptor.observation`→notice 每個 receptor 至多 2 s 一次（merge，correlation `receptor:<id>`）；companion.* 表面 receptor 不投影（不自我回音）；`dragged` 輸入→`companion.click{companion-dragged}` 每 instance 至多 1/s；`hover-entered`→`companion.pointer` 每 instance 30 s 一次。非安全 runtime 投影 priority 40、AI 請求 30。桌面 instance 的 presence heartbeat（`/v1/presentation/hello`，20 s）逾期即視為斷線（pending→uncertain、generation+1、發 `character.instance{connected:false}`），視窗必須重新 `/v1/character/hello`。

**協商完成後的真相補投**：投影只送給「投影當下已連線」的 instance，所以剛協商完成的角色（首次連上或重連）如果此刻正在**緊急停止**中，Runtime 會立刻單獨補投一則 `emergency`（correlation `emergency-stop:<instanceId>`，稽核 `character.estop-resync`），不必等下一次 estop 轉換。外部 adapter 沒有第二條路可查（adapter token 不能讀 `/v1/status`），所以這條補投是它知道「現在是緊急狀態」的唯一保證。

## 12. Reference adapters

| Adapter | 型態 | 宣告能力 | 用途 |
|---|---|---|---|
| `shu-rig`（小樞 v3） | in-process、參數化 rig＋遊玩場 | 全部 visual.*、audio.speech／effect、input.*、multiCharacter、scene、rollCall、gameplay.* | 完整 Reference Implementation；36 表情與遊戲功能不變 |
| `sprite`（舊 v1／v2 pack） | in-process、sprite sheet | visual.presence／expression（variants=animations）、visual.gaze（有 anchors 時）、input.click／drag／drop／text／fileDrop | 舊 Character Pack 相容層 |
| `text`（最小文字角色） | in-process、DOM 文字 | visual.presence、visual.textBubble、audio.effect（可選）、input.click／text | 證明協定不依賴 rig；也是其他 adapter 停用／崩潰時的可信 fallback（`FALLBACK_ADAPTER_ID`） |
| `shape`（`ref-shape`，幾何角色） | in-process、DOM 圓形 | visual.presence、visual.expression（variants：idle／notice／play／rest／work／think／acknowledge／wait／greet／sleep）、input.click | 第二個 Reference Character：證明「加一個角色不必改協定核心」。顏色隨 intent 家族、`play` 脈衝一次、`notice` 位移、其餘靜止；Reduced Motion 只變色。不支援 audio／gaze／particles／遊玩，安全 intent 不宣告 → 一律落 `system.text`。加入方式見 `docs/aip/reference-character.md` |
| WebSocket 外部 adapter fixture | external、`examples/character-adapters/text-adapter.mjs` | 純文字（無 expression）；只回 accepted／started／completed | 證明外部 transport；CLI E2E 使用 |

## 13. 測試矩陣（必須存在的測試）

| 層 | 檔案 | 覆蓋 |
|---|---|---|
| Rust 權威實作 | `crates/interaction-character/tests/{manifest,negotiation,gateway,conformance}.rs`＋各模組單元測試（116） | manifest 驗證／惡意 manifest／路徑穿越／migration；協商（版本、能力、fallback、reduced motion、純聲音、零能力）；lifecycle／ack 誠實／cancel 冪等／重複／過期／世代／crash／有界佇列／payload 上限／偽造 verified／emergency 搶占／速率預算共用；`conformance.rs` 對每份內建與第三方 manifest（`CPP_CONFORMANCE_MANIFESTS`）驗 20 個 intent、安全 intent 不遺失（含「adapter 用任何非 completed 終態結束安全 intent 都必須補 system.text」）、claimed≠verified、emergency floor 100；`intent_capabilities_golden.rs` 與 TS `character-protocol.test.ts` 對同一份 golden 比對 §3.4 的 intent→能力表，擋住權威／鏡射漂移 |
| Rust runtime 接線 | `crates/interaction-runtime/tests/character_loop.rs`（19）、`crates/interaction-api/tests/api_e2e.rs`（WS fixture＋HTTP 速率限制）、`crates/interaction-cli/tests/cli_e2e.rs` | hello／re-hello 世代、Reduced Motion 一路協商到回執、§11 投影每一列、receipt 結算 presentation receipt、input 正規化、adapter token 分權、WS 握手／估限／撤銷、斷線／重新協商 → uncertain＋system.text、外部 adapter 不得偽造人類互動、估停期間連線的補投、AI 呈現命令只由桌面結算 |
| TS 鏡射 | `apps/interaction-desktop/src/test/character-{protocol,gateway,adapters,manifests,shu-adapter}.test.ts`、`companion-gateway-wiring.test.ts`、`companion-imported-characters.test.ts`、`regressions-run2-companion.test.ts` | 同上的 TS 端＋三個 reference adapter＋角色視窗接線＋匯入角色＋對抗審查回歸 |
| 外部 transport | `scripts/v03-cli-e2e.sh`「Character Protocol」段（Node fixture 走 WebSocket，標示模擬 adapter） | 註冊→WS hello/negotiate→intent→receipt→撤銷；human token 上 WS 被拒、adapter token 打人類路由被拒 |
| 可信 host | `apps/interaction-desktop/src-tauri/src/{host_safety,character_store}.rs` 單元、`src/test/overlay.test.tsx` | overlay 只由 Rust 驅動、匯入驗證、資產 magic bytes、路徑再檢查 |


Manifest schema validation、protocol version negotiation、capability negotiation、unknown capability、fallback selection、
command lifecycle、ack／completion 誠實性（acknowledged→uncertain）、cancel idempotency、duplicate messageId、expired message、
reconnect generation、adapter crash、bounded queue、payload size limit、malicious manifest、path traversal、偽造 verified、
emergency priority、reduced-motion negotiation、pure-audio／no-visual fallback、legacy pack migration、外部 transport fixture。
