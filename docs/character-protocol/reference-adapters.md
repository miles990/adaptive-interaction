# Reference Adapters 導覽（CPP 1.0）

四個 reference adapter 各代表一種型態；它們共用同一份契約（[`README.md`](README.md)），Runtime 對它們一視同仁。

| Adapter | 型態 | 程式 | 能力宣告 | 證明什麼 |
|---|---|---|---|---|
| 小樞 `shu-rig` | in-process、參數化 2D rig＋遊玩場 | `apps/interaction-desktop/src/character/adapters/shu.ts`＋`shuTables.ts` | 全部 `visual.*`、`audio.speech`／`effect`、7 個 `input.*`、`multiCharacter`、`scene`、`rollCall`、`gameplay.*`；36 正式表情為 variants | 完整 Reference Implementation；Runtime 不再引用耳朵／尾巴／表情名 |
| `sprite` | in-process、sprite sheet（舊 v1／v2 pack 相容層） | `src/character/adapters/sprite.ts`＋`spriteIntents.ts` | `visual.presence`／`visual.expression(variants=animations)`／（有 anchors 時）`visual.gaze`、5 個 `input.*` | 舊 Character Pack 零設定遷移；沒有的動畫誠實 `substituted`，`failed`→`blocked` 永不落到 success |
| `text` | in-process、DOM 文字 | `src/character/adapters/text.ts`＋`lines.ts` | `visual.presence`、`visual.textBubble`、`input.click`、`input.text` | 最小文字角色；協定不依賴 rig；也是小樞停用／崩潰時的可信 fallback |
| WebSocket 外部 fixture | external-process、Node ≥ 22、零依賴 | `examples/character-adapters/text-adapter.mjs`＋`text-adapter.manifest.json` | `visual.presence`、`visual.textBubble`、`input.text` | 外部 transport 閉環（hello→negotiate→intent→receipt）；CLI E2E 使用，標示為**模擬 adapter（fixture）** |

## 1. 小樞如何從「寫死的唯一角色」變成 Reference Adapter

之前：Runtime 的 `PLAYABLE_ANIMATIONS` 寫死小樞 rig 的動畫名；`machine.ts` 把 `agent.session.state` 直接映成
`wait-codex`／`device-hello` 等表情 id；`director.ts` import 小樞表情表；`CompanionApp` 直接操作 `StageRenderer` 的
19 個方法；provider id 是 `provider.companion.shu`。

現在：

1. **Manifest**：`public/characters/shu-maid{,-dusk,-sakura}/manifest.json`（`entrypoint: builtin shu-rig`，三個 palette 是
   `variants`，`pronouns: 她`）。舊 `character-rig 2.0` manifest 由 `migratePackToManifest` 自動轉成同一份。
2. **協商**：`ShuCharacterAdapter.negotiate(hello)` 從 manifest 宣告能力；Runtime 據此算出 20 個 intent 的解析
   （小樞全部 `exact`）與 AI 可點播的動畫集合（`PresentationBridge::playable_animations()`＝協商到的
   `visual.expression.variants` ∪ 非安全 canonical intents − 真相狀態 deny-list）。
3. **Intent → rig**：`shuTables.ts::shuExpressionPlan(intent, truthState, variant)` 是唯一知道「`claim-completed`＝
   `success-claimed`（frameSlice [0,1]）、`verified-success` 只在 `truthState==="verified"` 才是 `success-verified`、
   `greet`＝`device-hello`、`notice(variant: device-offline)`＝`device-lost`」的地方。Runtime 端只有 §11 的語意投影。
4. **Director 引擎中立**：`InteractionDirector(tuning, tables)`——ambient 變體、反應表、落地表情、睡眠集合、個性權重全部
   由 adapter 注入（`SHU_DIRECTOR_TABLES`／`SHU_LANDING`／`SHU_VARIANT_WEIGHTS`）；`isPlayable` 也是注入的，
   真相狀態表情不可被 ambient 點播的防線因此仍在，但不再 import rig。
5. **回執**：`perform()` 依 timeline 送 `accepted → started → completed`；被更高優先的本機演出擠掉回
   `cancelled{reason:"preempted"}`；`cancel` 冪等；`hide/suspend` 真的 `StageRenderer.pause()`（rAF／物理／音效停）。
6. **Provider**：`provider.companion.desktop`，顯示名跟著目前角色（「桌面角色：小樞（Presentation）」；未 hello 前
   「桌面角色（尚未連線）」）。

遊戲功能（6 種玩具、2D 物理、3 使魔、場景、Roll Call、四種落地、hover 氣泡）全部留在 `GameplayExtension` 內，
是**可選擴充**：`playfield.test`／`rig.test`／`gameFeel.test` 全數重跑通過。

## 2. 桌面視窗的兩層 Gateway

```
Runtime（Rust Gateway）──character.intent（SSE／Tauri IPC）──▶ CompanionApp
                                                             │  CharacterGateway（TS，in-process）
                                                             ├─ ShuCharacterAdapter（primary-companion）
                                                             ├─ SpriteCharacterAdapter（舊 pack）
                                                             └─ TextCharacterAdapter（fallback）
CompanionApp ──receipt（POST /v1/character/receipts）／event（POST /v1/character/events）──▶ Runtime
```

- Runtime 把桌面視窗視為**一個** instance（`desktop-companion`）；視窗內的 TS Gateway 再分派給本機 adapter。
- 回執只轉發主 instance 的（去掉 `@instance` 後綴，帶 Runtime 的 generation）；本機的 `~idle`／`~r` 衍生命令不上傳。
- 沒有 `characterProtocol`（舊 daemon）時走 legacy `mapRuntimeEvent` 路徑（程式內標示 legacy）。
- 角色載入失敗／adapter 崩潰 → `TextCharacterAdapter`＋固定文字「角色載入失敗，改用文字顯示」，顯示在
  CompanionApp 自己擁有的可信 DOM 元素（`.companion-system-text`），不在任何 adapter 內。`character.system-text`
  事件也顯示在同一元素。

## 3. 文字角色（`plain-text`）

`public/characters/plain-text/manifest.json` ＝ `buildTextCharacterManifest()`。在角色頁選它，整個系統照常運作：
工作中／等待同意／聲稱完成／已驗證／失敗／未知／緊急停止都以固定文案一行呈現（綠勾只在 verified），
Runtime 的安全事件從未遺失。這證明「不支援某個動作的角色」與「純文字角色」都是一等公民。

## 4. 外部 WebSocket fixture 走一遍

```bash
interact-ai serve &                                   # 或桌面 app 內嵌 runtime
interact-ai character adapters add --name "文字 adapter" \
  --manifest examples/character-adapters/text-adapter.manifest.json   # 印出 adapterId＋token（只此一次）
INTERACT_AI_CHARACTER_TOKEN=<token> node examples/character-adapters/text-adapter.mjs
# 另一個終端：
interact-ai character instances                       # 看到 adapter:<id>（role familiar）
interact-ai character intent notice --message "hi"    # 非安全 intent；fixture 印出一行並回 completed
interact-ai events --seconds 5 --json | grep character.receipt   # /v1/events 會先回放最近事件再跟隨 5 秒
interact-ai emergency-stop --reason test              # fixture 收到 intent emergency（priority 100）
interact-ai character adapters revoke <adapterId>     # goodbye＋斷線；token 立即失效
```

`scripts/v03-cli-e2e.sh` 的「Character Protocol」段自動化了上面的閉環（含 human token 上 WS 被拒、adapter token
打 `/v1/status`／`/v1/emergency-stop` 被拒 403、手動送安全 intent 被拒）。

## 5. 匯入自己的角色（純資料）

角色頁「更換或加入角色 → 匯入」貼上 manifest JSON（可附 sprite sheet base64）。Tauri host 的 `character_import`
用 Rust 驗證器（`interaction-character`）＋magic bytes＋大小上限＋路徑再檢查後，存到
`<home>/state/characters/<characterId>/`；只允許 `adapterKind: in-process` 且 `entrypoint` 在白名單
（`shu-rig`／`sprite`／`text`）。外部可執行 adapter 一律走 `character adapters add`（人類明確授權、token 分權）。
