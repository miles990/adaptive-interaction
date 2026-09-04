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
| `needs-reconfirmation` | 需要重新確認裝置 | 有裝置現在連著這台電腦，但還不是角色同步的成員（撤銷過的裝置重新連上來也算）；要再同步角色，必須在手機上重新確認一次。 | warn |
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
影響人類看到的答案（§8 必測清單要防的正是這件事）。Runtime 目前還沒有把協商結果投影到
`GET /v1/character-session`／`/diagnostics`（`MemberView` 只有 party／role／presence／lastSeenAt），
所以實務上多數情況會落在 `capability-unknown`——**不猜**：既不給綠勾，也不誣賴裝置做不到。
桌面認得 `members[].unsupportedIntents`（數字或陣列）與 `members[].negotiated.intents`
（intent → `exact`／`unsupported`）兩種形狀，Runtime 補上任一種就會自動生效。

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
- 「撤銷」不是回到空狀態——是 `needs-reconfirmation`（需要重新確認裝置）。
- 認不得的 presence 不猜：退回 `syncing` 並把 `known` 標成 `false`，補充句改成
  「有裝置回報了這台電腦不認得的狀態；在弄清楚之前都當成尚未完成，不會當成已同步。」

**刻意的行為（不是缺陷）**：撤銷過一台手機之後，即使裝置清單已經空了，卡片仍然顯示
`needs-reconfirmation`（需要重新確認裝置），直到你重新配對。理由是它講的是事實——那台裝置的
授權被撤銷了，角色要再同步就得再確認一次——而不是把「我把它移除了」偷偷說成「一切正常」。
`no-device` 只留給**從來沒有裝置被撤銷過**的那台電腦。

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

**桌面端刻意不做接收端 hash 核對。** JS 的 number 留不住數字字面（Runtime 的 `0.0` 在 JS
重新序列化之後是 `0`），重算出來的 canonical JSON 不可能與 Rust 端逐位元組相同——做了就是
一個永遠亮著的假警報。不一致時的處理是「重新取一次完整快照對齊」，判斷依據是 revision
單調遞增與 `baseRevision` 相符。這些字眼一個都不會出現在畫面上。

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
