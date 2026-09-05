# 一般模式的角色同步 UX（v0.6.0）

> 契約來源：`docs/aip/character-session.md` §11（文案表）、`docs/aip/transport-bindings.md` §2（四條路由）。
> 權威投影：`apps/interaction-desktop/src/statusProjection.ts`（`projectCharacterSession` 等純函式）；
> 呈現：`apps/interaction-desktop/src/components/CharacterSyncCard.tsx`。
> 這份文件寫的是**使用者看到什麼**，以及哪些東西一般模式**永遠不會**看到。

## 1. 五個主入口不變

角色同步**不是**第六個一級入口。它住在第二個入口（「目前角色」的動態名稱）的頁內區塊「同步」裡。
主入口永遠恰好五個：現在／〔角色名〕／工作／連接與權限／更多。
守門測試：`src/test/regressions-v06-general-mode.test.tsx`（`SIMPLE_NAV` 長度與 id 鎖死）。

| 入口 | 角色同步在這裡出現的形式 |
|---|---|
| 現在 | 不出現（首頁只回答三個問題，不加第四張卡） |
| 〔角色名〕 | 「同步」卡：狀態一句話＋裝置清單＋最近互動＋（進階）連接診斷 |
| 工作 | 不出現 |
| 連接與權限 | 手機卡上多一行「角色同步：…」——連上 ≠ 已同步 |
| 更多 | 不出現（進階模式的開關仍在「更多 → 進階」） |

## 2. 文案表（一字不改）

十一種狀態窮舉（`CHARACTER_SYNC_PROJECTION`，`satisfies Record<CharacterSyncState, …>`：
少一個狀態就 typecheck 失敗，不會靜默退化成把技術值印到畫面上）。

| 狀態 | 主要句子 | 補充 | 顏色 |
|---|---|---|---|
| `synced` | iPhone 已連接，角色狀態已同步 | 手機上的角色和這台電腦看到的是同一個狀態。 | ok（唯一的綠） |
| `reconnecting` | iPhone 正在重新連線 | 連線斷了一下，正在接回來；這段時間的互動不會補播。 | pending |
| `offline` | iPhone 暫時離線 | 手機現在收不到角色狀態，也送不出互動；接回來之後才會重新對齊。 | warn |
| `partial-capability` | 部分能力目前不可用 | 這台裝置接上了，但它做不到角色的部分表演；做不到的不會假裝做到。 | warn |
| `capability-unknown` | iPhone 已連接，能力核對中 | 狀態對齊了，但還沒確認這台裝置演得出哪些表演；在確認之前不要當成完全同步。 | pending |
| `syncing` | 同步尚未完成 | 還在對齊角色狀態；在這之前不要把畫面上的樣子當成最新的。 | pending |
| `unrecoverable` | 無法恢復，請重新連接 | 連續好幾次都對不齊角色狀態，需要你重新連一次裝置。 | bad |
| `needs-reconfirmation` | 需要重新確認裝置 | 有裝置現在連著這台電腦，但還不是角色同步的成員（撤銷過的裝置重新連上來也算）；要再同步角色，必須在手機上重新確認一次。detail 指出是哪一台（只用顯示名稱）。 | warn |
| `local-only`（v0.6.x） | 目前只在這台電腦使用 | 之前移除過的手機不會自動回來；要再用手機時重新配對一次就好。這是移除之後的正常狀態，不是故障。 | muted |
| `store-issue`（v0.6.x，取代 `store-reset`） | 同步紀錄暫時存不下來 | 這一輪的同步紀錄存不下來（或目前寫不進去），重新啟動之後會再試；角色和裝置的連線不受影響。「曾經重建過」只是灰色附註，不是這個狀態。 | warn |
| `no-device` | 尚未連接 iPhone | 目前只有這台電腦在陪你；連上手機之後才會有東西可以同步。 | muted |
| `disabled` | 角色同步目前關閉 | 這台電腦沒有啟用角色同步；其他功能不受影響。 | muted |
| `store-reset` | 角色同步紀錄曾損毀，已重新開始 | 已重新連接的裝置會重新同步；不影響角色本身。 | warn |

判定順序（先擋住「不能相信」的情況，最後才談成功）：
關閉 → 連續讀不到 → 這一次讀不到 → 認不得的回報 → 紀錄曾損毀 → 有 online 成員
（已知做不到 → `partial-capability`；另有裝置**現在連著**卻不是成員 → `needs-reconfirmation`；
拿不到協商結果 → `capability-unknown`；全部確認演得出來 → `synced`）→ reconnecting →
offline → 需要重新確認 → 沒有裝置。

綠勾只在「所有 online 成員都確認演得出來、而且沒有別的裝置連著卻沒同步」時出現。
`needs-reconfirmation` 在有 online 成員時只看「現在連著卻不是成員」，**不看**「曾經有裝置被撤銷過」
——後者是歷史事實（provider 列會永遠留著 revoked），拿它壓過一台真的在線的裝置就成了
§3 說的那種永遠亮著的假警報；真的需要重新確認的裝置只要連上來，就會以「連著但不是成員」
的身分被算進去。

`partial-capability` 與 `capability-unknown` 的判定來源是**協商結果**，不是成員自報的 `role`：
`role` 是裝置在 capability 宣告裡自己填的，拿它當能力結論等於讓 renderer capability spoofing
影響人類看到的答案（§8 必測清單要防的正是這件事）。Runtime（v0.6.0 起）已把協商結果投影進
`GET /v1/character-session` 的 `members[].unsupportedIntents`（永遠是陣列；全支援＝空陣列）與
`/diagnostics`（`crates/interaction-session/src/state.rs` `MemberView`；測試
`crates/interaction-session/tests/session.rs::negotiated_unsupported_intents_are_projected_into_members`）。
只有在拿不到協商結果（例如 restore 之後成員尚未重新協商）時才落在 `capability-unknown`——**不猜**：
既不給綠勾，也不誣賴裝置做不到。桌面另外仍認得 `members[].negotiated.intents`
（intent → `exact`／`unsupported`）這種形狀。

`store-reset` 的判定來源是 Runtime 診斷的 `storeNote` 不是 null（持久化檔讀不回來、已隔離、
epoch 已 +1）。**不靜默**：它排在「已同步」之前，因為那一刻技術上也許真的同步著，但綠色徽章
讀起來是「一切正常」，會把「你的裝置得重新對齊一次」蓋掉。它也不是緊急狀況（不給紅色），
講的是紀錄而不是角色。緊急停止的固定安全句永遠壓過它（§7）。

> 一般模式**會**讀 `GET /v1/character-session/diagnostics`（`storeNote` 只有這一個來源），
> 但一個數字都不會顯示：「連接診斷」收合區塊仍然只在進階模式出現。
> 守門測試同時斷言「人話有」與「`.tech-details` 不存在」。

**成員行**：已連接／重新連線中／離線／狀態不確定。
**最近互動**：摸了摸角色／輕拍了角色／撫摸了角色／按著角色不放／和角色互動了一下／請角色休息一下。

## 3. 空狀態 ≠ 成功

「沒有裝置」是中性狀態，不是成就。`no-device` 的顏色是 muted，卡片上**不會**出現任何綠色徽章；
文案也不寫成「一切正常」。同樣地：

- 「讀不到權威狀態」不是「已同步」——一律 `syncing`（同步尚未完成）。
- 「離線」不是「沒有裝置」——成員還在，只是這台裝置現在收不到。
- 「撤銷」不是回到空狀態，也不是永遠亮著的待辦：那台裝置**還連著**就是 `needs-reconfirmation`（需要重新確認裝置）；
  使用者主動移除了全部手機、清單空了，就是中性的 `local-only`（目前只在這台電腦使用），文案仍明說它不會自動回來。
- 認不得的 presence 不猜：退回 `syncing` 並把 `known` 標成 `false`，補充句改成
  「有裝置回報了這台電腦不認得的狀態；在弄清楚之前都當成尚未完成，不會當成已同步。」

**v0.6.x 的語意修正**：v0.6.0 在裝置清單已空時仍顯示 `needs-reconfirmation`（當時記為刻意行為）。這在使用者
主動移除全部手機之後變成一個永遠亮著、無事可做的警告。現在零裝置＋只剩歷史撤銷＝`local-only`（「目前只在這台電腦
使用」，中性、附「不會自動回來、要再用就重新配對」）；只有撤銷過的裝置又連上來（連著但不是成員）才回到
`needs-reconfirmation`。撤銷的安全效果不變（Runtime 不會自動重新授權；provider 列仍永遠留著 revoked）。
`no-device` 留給從來沒有裝置被撤銷過的電腦。每一態另有穩定的下一步 action id（見 `character-session.md` §11）。

## 3.5 卡片怎麼知道現在的狀態（不是每秒重問）

同步卡的權威狀態來自兩個地方，順序固定：

1. **首次載入**（以及使用者按「重新檢查」、收到接不上的補丁時）呼叫
   `GET /v1/character-session` 取一份完整快照。這條路由**會消耗一個 session sequence**，
   所以不能每則 runtime 事件都打一次。
2. 之後靠 SSE `character.session.state`：`snapshot` 整份取代本地副本，`patch` 在 epoch 相同
   且 `baseRevision` 等於本地 revision 時以 RFC 7396 merge patch 套上去。revision 沒有前進的
   訊息一律忽略（不倒退）。

裝置名稱、來源清單與診斷（`storeNote`）是另一組：節流成最小間隔 2 秒的 trailing 重取，
不隨每一則 runtime 事件重打。

**桌面端會做接收端 hash 核對**（AIP §6；`src/aip/canonical.ts`＋`src/aip/sessionClient.ts`）。
JS 的 number 留不住數字字面，但 canonical 規則可重印（f64 路徑由 codegen 從跨語言 fixture
manifest 產出，`pnpm aip:check` 是漂移 gate），三端共用的 `stateHashes` fixtures 逐位元組核對過。
對不上就**不套用**，改走 `POST /v1/character-session/resume` 重新對齊；連續對齊失敗達 3 次
升級成「無法恢復，請重新連接」（誠實說狀態未知，不是無限重試）。revision／epoch／hash／
`alignment.*` 計數這些字眼一個都不會出現在一般模式的畫面上（只有進階模式的「連接診斷」）。

## 4. claimed ≠ verified：綠勾只給真的

同步卡的十一種狀態裡只有 `synced` 是 `ok`（綠）。這條規則和工作狀態的誠實階梯是同一條：
`claimed-completed`（對方說做完了）永遠不是 `verified`（你檢查過了），
`projectWorkState` 沒有任何路徑能把 claimed 升級成 verified。
角色同步不會、也不能改寫這一層——它同步的是「角色現在是什麼語意狀態」，不是「工作有沒有做完」。
`task.*` 的真相由 Runtime 轉錄進 session，session 只轉錄、不推論。

**送出 ≠ 生效**：桌面角色被點一下時送出的語意事件，只有 Runtime 回 `applied` 才代表權威狀態真的改了；
`rejected`／`expired`／不知道，介面一律照實說，不當成成功。

## 5. 一般模式看不到的東西

以下只在**進階模式**（更多 → 進階）的「連接診斷」收合區塊出現，一般模式一個字都不會有：

`revision`、`sequence`、`sessionEpoch`、`eventLog`、各種 counters、`storeNote`、
schema 版本、transport／token／provider id、裝置識別碼、原始 payload 或信封。

裝置在畫面上永遠用**名稱**稱呼；名稱查不到時用中性的「一台裝置」，**絕不**退回裝置識別碼。
守門測試同時做正反斷言（該有的人話有、不該有的技術詞一個都沒有）：
`src/test/statusProjection-session.test.ts`、`src/test/character-sync-card.test.tsx`、
`src/test/regressions-v06-general-mode.test.tsx`。

## 5.5 未解決停止（v0.6.x）

「感測不靜默」的另一半：一個感測來源被移除時還在擷取，之後**沒有任何人、也沒有任何裝置**
確認它停下來。這種紀錄離開了 `activeSensors` 即時清單，但沒有結論——不能因此從畫面上消失。

> 來源：Runtime `sensor_source.rs` 的 `UnresolvedStop`（`status.unresolvedStops`，空的時候
> 後端不序列化這個鍵）＋`GET /v1/sensors/unresolved`。
> 投影：`src/statusProjection/unresolvedStops.ts` 的 `projectUnresolvedStops`。
> 呈現：狀態列 `src/components/UnresolvedStopsBanner.tsx`、連接與權限
> `src/pages/connect/UnresolvedStops.tsx`、狀態列選單 `src-tauri/src/tray.rs`。

它**不是**這三件事，文案一律不得這樣寫：

| 它不是 | 為什麼 |
|---|---|
| 「還在感測」 | 沒有任何證據說它還在跑；正在跑的東西在 `activeSensors`／感測橫幅。 |
| 「已停止」 | 沒有任何來源確認過。這一區的存在理由就是「不知道」。 |
| 歷史紀錄 | 歷史在稽核裡。這張表回答的是「現在還有哪些事沒有結論」。 |

**三個出現的地方**（同一份投影，三處文字一致）：

- **控制中心狀態列**（頂端）：一行摘要「有 N 筆感測停止沒有人確認，到「連接與權限」逐筆看。」＋前往按鈕。
- **狀態列選單**（tray，Rust 直接算）：`系統狀態：…｜感測停止待確認 N`。感測中與未確認同時存在時兩段都在。
  它不叫出可信 overlay——overlay 只講「此刻正在發生」的事（緊急停止、正在感測、連不上）。
- **連接與權限 → 立即停止與撤銷**：逐筆一行，每一行說得出是哪一台、哪一種感測、多久以前的事。

**逐筆的人話**：`〔名稱〕的〔感測種類〕：〔相對時間〕離開使用中清單，沒有人確認過它。`
名稱只用後端給的人話名稱（`sourceLabel`）；沒有就說「某個裝置」，**絕不**退回 `sourceId`。
感測種類走共用的 `sensorKindLabel`（認不得的說「其他感測器」）。相對時間讀不出來就說「時間不明」。
清單有界（最多 20 筆），其餘誠實寫成「…還有 N 筆沒有列出來」。

**人為確認是二段的**，而且第二段的按鈕文字自己就說得清楚是誰在確認：

1. 「我確認它已經停了」
2. 「確定：這是你的確認，系統沒有收到裝置的回覆」

送出後的回報同樣再說一次（「已記下你的確認（這是你的確認，系統沒有收到裝置的回覆）。」）。
後端記的是人類的決定：`POST /v1/sensors/unresolved/{sourceId}/dismiss` 的回應 `confirmedStopped`
永遠是 `false`，而且解除**一定要指名世代**，才不會把同 id 的新一筆一起清掉。失敗不得靜默，
也不得說成已經處理掉（「沒有記下你的確認（…）：這一筆還在，請再試一次。」）。

**一般模式不外洩**：`sourceId`、`generation` 與原始感測種類 id 只拿去呼叫 API，一個字都不進畫面。
守門測試：`src/test/unresolvedStops.test.tsx`、`src/test/general-mode-no-technical-terms.test.tsx`（X5）、
`src-tauri/src/host_safety.rs`／`tray.rs` 的單元測試。

## 5.6 裝置「重新連線中」（v0.6.x）

宣告式裝置（Serial／MQTT／BLE）被重新啟用時，Runtime 不會把狀態直接跳成「可用」：它把整份宣告
重新註冊一次，實體連線與能力宣告要等握手成功才回得來，所以狀態**誠實地留在 `disconnected`**。
只印「未連線」會讓使用者以為沒有人在處理；失敗之後狀態也一樣是 `disconnected`，兩者必須分得出來。

> 來源：provider 的 `detail.warnings[]`（Runtime `providers.rs` 的 `REBINDING_WARNING`／
> `rebind-failed: <原因>`）。投影：`src/statusProjection/provider.ts` 的 `projectProviderConnection`。

| 情況 | 一般模式看到 | 補充句 |
|---|---|---|
| 重新綁定中 | 重新連線中（pending） | 正在重新連上這台裝置，它的能力還沒有回來；連上之後才會顯示為可用。 |
| 沒有重新連上 | 沒有重新連上（bad） | 這台裝置沒有重新連上，它的能力還沒有回來；請檢查裝置與接線，再重新啟用一次。 |
| 沒有重新連上（要重開） | 沒有重新連上（bad） | 這台裝置沒有重新連上：要重新啟動系統之後才能再試一次，它的能力還沒有回來。 |
| 沒有這些記號 | 照原本的生命週期標籤（未連線／可用／…） | — |

失敗的**原始原因是英文技術訊息**（例如 `the device could not be rebuilt: serial port busy`），
只在進階模式以「原始：…」補上；一般模式只有上表的人話。

「重新啟用」按下去之後可以誠實說出口的一句話是
**「已允許重新連線，正在重新綁定」**（`PROVIDER_REENABLE_MESSAGE`）——不是「已啟用」：
按鈕只是**允許再連一次**，連上與否要等握手。

> 目前控制中心**沒有**「重新啟用 provider」的按鈕（provider 狀態轉換只有 CLI／HTTP
> `POST /v1/providers/{id}/transition` 走得到），所以這句文案目前只有常數與守門測試，
> 畫面上還沒有觸發點。加上按鈕時直接用這個常數，不要另寫一句。

## 6. 模擬 iPhone（fixture）的標示

瀏覽器 journey（`e2e/character-session.spec.ts`）用的是
`crates/interaction-runtime/examples/fake_iphone.rs`——**模擬 iPhone（fixture）**，程序外的假手機，
不是 iPhone 真機。

- fixture 的裝置名稱本身就是「模擬 iPhone（fixture）」，投影**原樣顯示**、不再加工，
  所以成員清單、最近互動、連接頁的手機卡上都自帶這個標籤。
- 狀態句子（例如「iPhone 已連接，角色狀態已同步」）是契約 §11 的固定文案，
  裝置名稱在旁邊的清單裡各自列出，兩者不互相冒充。
- 截圖存在 `docs/assets/v06-evidence/`（寬視窗與 390px 各一組），檔名與說明一律標示 fixture。
- **iPhone 真機的角色同步驗收目前為零。** 任何文件都不得把上面這些寫成真機證據。

## 7. 緊急停止

緊急停止中，同步卡會多一句固定安全句：
「緊急停止中：角色已停止表演，解除前不會接受任何互動。」

**安全狀態壓過同步狀態**：這一句在的時候，徽章一律不是綠色（即使同步本身「技術上」還好好的）。
句子本身照實不改——已同步就是已同步——但綠色會讀成「一切正常」，和正下方的安全句互相矛盾。

這一句由可信的 host 介面顯示，角色、Character Pack、外部 adapter 都無法覆寫或隱藏它。
同一時間任何裝置送來的互動事件都會被 Runtime 拒絕（`rejected{scope-denied}`），
解除只能由人走安全流程，**不會**在重啟後自動恢復。
