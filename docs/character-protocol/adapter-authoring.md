# Character Adapter 撰寫指南（CPP 1.0）

> 讀這份之前先看 [`README.md`](README.md)（唯一契約）。本指南回答「怎麼做」；契約回答「必須做什麼」。
> 兩種接法：**in-process**（TypeScript，在桌面角色視窗內執行；最低延遲、可用游標／拖曳／物理）與
> **外部 adapter**（任何語言，WebSocket wire protocol；遊戲引擎、外部桌面程式、遠端顯示器）。
> 兩種接法收到的訊息完全相同（§8 wire messages）；差別只在傳輸。

## 0. 你永遠拿不到什麼

不論哪一種接法，adapter **拿不到**：human token、agent token、policy／consent 的修改權、解除 Emergency Stop、
`verified` 判定（你只能在 `truthState === "verified"` 時「畫」綠勾，不能自己說 verified）、對話內容（只有 intent
與 ≤200 字的 `presentationHints.message`）、原始游標軌跡、檔案系統（file-drop 只有 metadata＋短效 grant）。
Runtime 的安全文字（緊急停止／被阻擋／結果不確定／感測使用中）由可信 host 保證顯示；你的角色可以「演」，但不能
「取代」或「隱藏」它們。

## 1. 建立一個最小角色（in-process，純文字）

最小角色不需要 rig、不需要表情、不需要聲音。內建的 `TextCharacterAdapter`
（`apps/interaction-desktop/src/character/adapters/text.ts`）就是完整範例；下面是它的骨架：

```ts
import type { CharacterAdapter, AdapterHost, ReceiptSink } from "../adapter";
import type { CharacterManifest, Hello, Negotiate, IntentEnvelope } from "../protocol";
import { intentLine } from "../lines";

export class MyTextAdapter implements CharacterAdapter {
  readonly manifest: CharacterManifest = {
    schemaVersion: "1.0",
    characterId: "my-text",                         // 穩定身分
    displayName: { "zh-TW": "文字角色", en: "Text" },
    version: "1.0.0",
    adapterKind: "in-process",
    entrypoint: { kind: "builtin", id: "text" },   // host 白名單：shu-rig / sprite / text
    assets: [],
    capabilities: {
      "visual.presence": { supported: true },
      "visual.textBubble": { supported: true, interruptible: true },
    },
    inputCapabilities: { "input.click": { supported: true }, "input.text": { supported: true } },
    channels: ["bubble"],
    states: ["idle", "line"],
    intents: [/* 你原生支援的 canonical intent；沒列的會走 fallback／system.text */],
    variants: [],
    locales: ["zh-TW"],
    securityRequirements: { network: false, executable: false, fileAccess: "none", audioOutput: false, microphone: false, camera: false },
    resourceLimits: { maxAssetBytes: 0, maxConcurrentCommands: 1, maxQueue: 32, maxFps: 0 },
    fallbacks: { intents: { play: "notice", sleep: "rest" } },
    compatibility: { protocol: "1.x" },
  };
  private host!: AdapterHost;
  private line = "";

  async initialize(host: AdapterHost) { this.host = host; }
  negotiate(hello: Hello): Negotiate { /* 用 manifest 產生 negotiate；hello.reducedMotion 可讀 */ … }
  show() {} hide() {} suspend() {} resume() {} reconfigure() {}
  perform(envelope: IntentEnvelope, sink: ReceiptSink) {
    sink({ messageId: envelope.messageId, status: "started" });
    const { text } = intentLine(envelope.intent, envelope.truthState, envelope.presentationHints?.message);
    this.line = text;                               // 真的做了什麼，就回什麼
    sink({ messageId: envelope.messageId, status: "completed", resolution: "exact" });
  }
  cancel(messageId: string) { /* 冪等：已終結就什麼都不做 */ }
  dispose() { this.line = ""; }
  onInput(cb) { /* 保存 cb；click 時呼叫 cb({ kind: "character.clicked" }) */ return () => {}; }
}
```

把 manifest 放到 `apps/interaction-desktop/public/characters/<characterId>/manifest.json` 並加進
`public/characters/index.json`，角色頁就會列出它。**沒有任何角色需要臉、手、尾巴或聲音。**

## 1.1 in-process adapter 的 host meta（`BuiltinAdapterMeta`，v0.6.x）

除了 manifest，in-process 的 builtin adapter 在 `character/adapters/index.ts` 註冊時還會宣告一份 **host meta**：
`hasPlayfield`、`variants`（外觀／配色 id）、`personas`（說話風格 id＋顯示名；上限 16）與 `playfieldControls`
（遊玩場設定 UI 的 React 元件）。這是 host 唯一知道「這個角色有哪些角色專屬設定值」的地方：

- 角色設定的匯入／匯出（`companion/settingsTransfer.ts`）以 **目標角色的 adapter meta** 驗證 `companionFamiliars.palette`
  ∈ `variants`、`companionPersona` ∈ `personas`；沒有宣告就拒絕（不會拿別的角色的允許值頂替）。
- 角色頁不再依 entrypoint 字面分岔：有 `hasPlayfield` 才掛 `playfieldControls`；`personas` 非空才顯示說話風格。
- 註冊期不變量：宣告 `playfieldControls` 卻沒有 `hasPlayfield` → 註冊當下 throw。
- 守門測試 `architecture-no-entrypoint-switch.test.ts` 掃 `companion/*`、`character/gateway.ts`／`negotiate.ts`
  **與頁面層**（`pages/CompanionPage.tsx`、`pages/character/**`），不得出現 `"shu-rig"`／pack id／部位名字面。

## 2. 如何宣告能力

`capabilities` 只宣告你真的做得到的事（§3.1 的 canonical id 或 namespaced custom id）。每個能力可以附帶
`variants`（你的表情／動畫 id 清單）、`interruptible`、`resumable`、`durationRange`、`reducedMotionBehavior`
（`static`／`reduced`／`unchanged`／`disabled`）。

- 沒有表情？不要宣告 `visual.expression`。協商會把需要表情的 intent 解析成 `substituted`（走 `visual.pose`／
  `visual.textBubble`／`audio.*`）或 `unsupported`；安全 intent 永遠會落到 Runtime 的 `system.text`。
- 純聲音／燈光角色：只宣告 `audio.speech`／`audio.effect`／`light.cue` 也能演：**只要你在 `intents` 裡宣告接得住**，
  工作／思考／等待／未知／取消／阻擋／失敗／聲稱完成／已驗證都會解析到這些通道（`exact`，`via` 是 `audio.*`／`light.cue`）。
  沒有在 `intents` 宣告的 intent 不會被假裝支援：安全 intent 落到 Runtime 的 `system.text`，非安全 intent 誠實 `unsupported`；
  也可以用 `fallbacks.capabilities`（例如 `"visual.expression": ["audio.effect"]`）把視覺 intent 導到聲音，結果是 `substituted`。
  兩種情形分別由 `negotiation.rs::pure_audio_character_expresses_work_wait_unknown_when_it_offers_them` 與
  `negotiation.rs::pure_audio_character_still_resolves_all_safety_intents` 證明。
- 自訂通道：`com.example.character.wings`（至少三段）。它會被接受但標 `nonSafety`——**不能影響安全搶占**。

## 3. 如何接收 intent

握手完成後，Runtime 只送語意 intent（§4.1 的 20 個），不送「左耳轉 18 度」。每個 envelope 帶：

- `intent`＋`truthState`（Runtime 決定，你不能改）；
- `priority`（安全 intent 已經被 Runtime 抬到 floor）；
- `interruptPolicy`／`resumePolicy`／`durationHint`；
- `presentationHints`（tone／message／variant——只是建議）；
- `expiresAt`（過期就不要播，回 `expired`）。

你的工作是把 intent 映射到自己的東西：小樞把 `work` 映射到「努力工作」表情＋胸口核心亮起；sprite 角色映射到
`act` 動畫；文字角色印一行字；LED 角色亮一顆燈。映射表放在你的 adapter 裡（例如 `adapters/shuTables.ts`），
Runtime 不知道也不需要知道你的部位名稱。

## 4. 如何回傳 started／completed／failed

```
accepted（Gateway 會替你發；你自己發也無妨）
  → started       你真的開始播了
  → completed     演完了（≠ 工作 verified）
  | cancelled     被搶占／被取消
  | failed        播不出來（Gateway 會把安全 intent 改走 system.text）
accepted → unsupported   協商說支援但執行期發現不行
accepted → expired       收到時已過 expiresAt
accepted → acknowledged  你收到了但**不會回報 completion**（Gateway 之後記成 uncertain，不會猜成 completed）
```

不要跳過 `started` 直接 `completed`（非法轉換，會被丟棄並記 audit）。`completed` 只能代表呈現，
永遠不代表 Agent 的工作已被驗證。

## 5. 如何送出 click／drag／text event

in-process：`onInput(cb)` 收到的 callback 呼叫 `cb({ kind: "character.clicked", payload: { x, y } })`。
外部：送 `{"type":"event","event":{ protocolVersion, eventId, characterInstanceId, generation, timestamp,
kind:"character.text-submitted", payload:{ text }, privacyClass:"personal" }}`。

Gateway 會正規化與節流：hover ≤ 4/s、drag ≤ 10/s（合併、8 px 量化）、佇列 64、絕對座標／檔案路徑一律丟棄。
`character.action-requested{action}` 只是**請求**——桌面（可信 host 表面）的事件會被 Gateway 轉成
`companion.quick-action` 觀察，仍要過 Runtime policy／consent；角色點擊永遠不會直接啟動 Agent、操作硬體或碰檔案系統。

**外部 adapter 的輸入事件不會變成觀察**：你送的 `event` 會被正規化、計入速率預算並留下稽核
（`character.input-not-observed`；HTTP 回 `{"decision":"audit-only","reason":"external-adapter-input-not-observed"}`），
但不會寫進 `companion.*` 受器，因此不會觸發配方、也不會被當成「使用者回應了」。
adapter token 不能呼叫 actuator，同理也不能合成人類互動——要讓使用者的操作進入 Runtime，請走桌面表面。

## 6. 如何處理 cancel、Emergency、Reduced Motion

- `cancel{messageId}`：停止該演出並回 `cancelled`；重複 cancel 回同一結果（冪等）；已終結的回
  `cancelled{alreadyTerminal:true}`。
- Emergency：你會收到 `intent:"emergency"`、`priority:100`、`interruptPolicy:"preempt"`。Gateway 已經把你正在播的
  非安全演出 `cancelled{reason:"preempted"}`。你要做的是**凍結一切遊戲動畫、停音效、擺出安全姿勢**；不能把
  emergency 演成慶祝，也不能因為「尾巴動畫不支援」而不演。安全文字由 host 顯示，你不必也不能覆寫。
- Reduced Motion：`hello.reducedMotion=true` 時，協商會依你宣告的 `reducedMotionBehavior` 把解析結果標成
  `reduced`；你的 `perform` 要真的靜態（不要只是變慢）。執行中切換時 host 會重新協商或呼叫 `reconfigure`。
  這個值由可信 host（桌面視窗）回報給 Runtime，Runtime 是唯一的主人；你回執裡的 `resolution` 只能比協商結果
  **更差**（誠實降級），回 `exact` 不會讓它變回 `exact`。

## 7. 如何加入自訂 channel

在 manifest 的 `channels` 加 `com.example.character.wings`，在 `capabilities` 加同名 custom capability。
`negotiated.acceptedChannels` 會包含它，`nonSafetyChannels` 也會（提醒你它不能參與安全搶占）。
`presentationHints.channels["com.example.character.wings"]` 可以帶建議參數（≤ 4 KB、經 schema 驗證）。

## 8. 如何使用 WebSocket adapter（外部程式）

1. 用人類 token 註冊，拿 adapter token（只會顯示一次）：
   `interact-ai character adapters add --name "我的引擎" --manifest my-character.manifest.json`
2. 連 `ws://127.0.0.1:8787/v1/character/ws?token=<adapter token>`（loopback；human／agent token 會被拒）。
3. 第一則訊息是 Runtime 的 `hello`；回 `negotiate`；收到 `negotiated` 後開始收 `intent`、回 `receipt`；
   每 15 秒互送 `heartbeat`；45 秒沒訊息會被視為斷線（pending 全部 `uncertain`，`generation+1`，重連後重新
   `hello`；舊 generation 的回執會被丟棄）。
4. 限制：單則 ≤ 64 KB、≤ 50 則/s、pending ≤ 64、outbound ≤ 32。

完整可跑的範例：`examples/character-adapters/text-adapter.mjs`（Node ≥ 22，零依賴）＋
`text-adapter.manifest.json`。`scripts/v03-cli-e2e.sh` 的「Character Protocol」段用它做閉環驗收（標示為
**模擬 adapter（fixture）**，不是真引擎）。

stdio JSON Lines 用同一批訊息（一行一則）；本版**不**自動啟動任何子程序（`entrypoint.process` 只是紀錄），
所以 stdio **只有規格、沒有實作**（沒有 host spawn、沒有 fixture、沒有 E2E）。相容表：WebSocket＝已實作、
In-process＝已實作、stdio JSON Lines＝規格已定／未實作（README §8.1）。

## 9. 如何從舊 Character Pack 遷移

不用改設定。舊 `character-pack` 1.0／1.1（sprite sheet）與 `character-rig` 2.0 由 `migratePackToManifest`
（TS）／`migrate_pack_to_manifest(json, &registry)`（Rust；v0.6.0 起走 host 註冊的 `MigrationRegistry`，
核心只內建 sprite，`character-rig` 由 `interaction-character-shu` 的 `RigPackMigrator` 提供；舊的
`migrate_legacy_pack` 已 `#[deprecated]`、只剩 sprite）自動變成 manifest：sprite 的動畫名成為 `visual.expression.variants`，
FALLBACKS 鏈成為 `fallbacks.intents`（例如 v1 沒有 `failed` 動畫 → `failed→blocked`，永遠不會落到 success）。
安全 intent 只會映到另一個安全 intent：v0.5.1 起遷移不再產生 `emergency→sleep`、`blocked→sleep`、`ask→notice` 這類映射，
缺美術的安全 intent 改走能力鏈或 `system.text`。
`DesktopPrefs.companionPack` 的 8 個舊 id 全部仍可用。匯入舊 pack JSON 時同樣自動遷移。

## 10. 如何在缺少某項能力時提供 fallback

`fallbacks.capabilities["visual.expression"] = ["visual.pose", "visual.textBubble"]`：表情做不到就換姿勢，再不行就
換文字泡泡。`fallbacks.intents = { play: "notice", sleep: "rest" }`：不會玩就「注意到」，不會睡就「休息」。
**安全 intent → 非安全 intent 的 `fallbacks.intents`（例如 `request-consent: "greet"`）會在 manifest 驗證階段被拒**（Rust／TS 同步；
匯入也擋）；`failed → blocked` 這種安全→安全的用法照舊。
Fallback 只會讓解析結果**變差**（`exact → substituted → reduced → unsupported`），不能把 `claimed` 變成
`verified`，也不能把安全 intent 變成 `unsupported`（安全 intent 最後一定落在 `system.text`）。

## 11. 如何測試與驗證 adapter

- in-process：`CharacterGateway` 可完全離線測（注入 `now()`），見 `src/test/character-gateway.test.ts` 與
  `src/test/character-adapters.test.ts`——照抄那些案例對你的 adapter 跑一遍（20 個 intent 全部解析、claimed≠verified、
  emergency 搶占、cancel 冪等、suspend 停 rAF）。
- 外部：先用 `interact-ai character intent notice` 手動送一個非安全 intent（CLI **拒絕**送安全 intent——安全
  truthState 只能來自 Runtime 事件），看 `interact-ai character instances` 與 `events` 裡的 `character.receipt`。
- Rust 端的權威驗證：`cargo test -p interaction-character`（manifest 驗證、協商、gateway 案例）；
  JSON Schema：`schemas/character-protocol.schema.json`。
- **第三方一致性測試**：`crates/interaction-character/tests/conformance.rs` 會對
  `examples/character-adapters/*.manifest.json` 與 `apps/interaction-desktop/public/characters/*/manifest.json`
  逐一驗收。要驗你自己的 manifest（不必放進這個 repo），用 `:` 分隔路徑（檔案或目錄都可以）餵給它：
  `CPP_CONFORMANCE_MANIFESTS=/path/to/my.manifest.json:/path/to/more cargo test -p interaction-character --test conformance`。
  它檢查四件事：manifest 通過驗證、20 個 intent 全部有解析結果且安全 intent 永不 `unsupported`、
  `claimed` 不會被換成 `verified`（含 `fallbacks.intents` 與變體名）、`emergency` 的 priority floor 仍是 100 且不會遺失。
  路徑不存在會直接失敗（不會靜默跳過）。
- 什麼算「已測試」：只有真的跑過閉環（hello→negotiate→intent→receipt）才會在連接頁顯示「已測試」；
  原始碼存在、編譯成功、fixture 通過都不算真機驗收。

## 12. 版本相容

`protocolVersion` 同 major 相容：未知欄位保留、未知 intent 回 `unsupported`、未知 event 被丟棄（記 audit）、
未知 capability 視為 custom。major 不同時握手被拒（`error{code:"protocol-version"}`），Runtime 不猜。
