# Character Session 持久化快照 fixtures

這些是 `<home>/state/character-session.json` 的**真實檔案內容**，測試把它們原樣放進一個
暫存 home 再讓 `CharacterSessionHost::open` 開機一次，所以「舊格式讀不讀得回來」是這一版
程式真的讀過的結果，不是模擬。

產生器與比對測試：`crates/interaction-runtime/tests/character_session_loop.rs`
（`character_session_fixtures_are_what_this_version_writes`）。

重生：

```bash
AIP_UPDATE_FIXTURES=1 cargo test -p interaction-runtime --test character_session_loop
```

## 檔案

| 檔案 | 是什麼 | 期望行為 |
|---|---|---|
| `v0.6.0-format0.json` | v0.6.0 會寫出來的快照：**沒有** `format` 鍵；桌面＋一台裝置成員、`lastInteraction`、`mood.intensity` 為 `0.0` | 還原成功＋遷移；原檔備份成 `character-session.json.pre-format-0` |
| `v0.6.0-dev-pre-unsupported-intents.json` | v0.6.0 開發期（`MemberView.unsupportedIntents` 這個欄位出現之前）寫下的快照：成員缺該鍵，`hash` 以檔案裡那份原始 `state` 計算 | 還原成功＋遷移（這就是 v0.6.0 已知限制 #21 的實例，當時會被判 `HashMismatch` 後隔離） |
| `future-format-99.json` | 宣告 `format: 99`、還帶了這個版本讀不懂的鍵 | **保留、不隔離、不覆寫**；store 進入 parked，session 以記憶體模式跑 |

`mood.intensity` 之所以固定放 `0.0`，是因為 serde_json 的 f64 必須寫成 `0.0`（不是 `0`）——
canonical hash 在 Rust／TypeScript／Swift 三邊對得上就靠這個書寫。

## 為什麼 `v0.6.0-format0.json` 等同 v0.6.0 寫出來的檔案

fixture 是用 HEAD 的程式產生的，再把 `format` 鍵拿掉（v0.6.0 的 `Snapshot` 沒有這個欄位）。
這樣做站得住腳，是因為**產生 fixture 當下**（commit `055d638`，本分支對 session／aip crate 的
程式碼變更之前）決定檔案內容的兩個 crate 的 `src/` 自 `v0.6.0` tag 起零差異：

```console
$ git diff --stat v0.6.0..055d638 -- crates/interaction-session/src crates/interaction-aip/src
$ echo $?
0
```

（`v0.6.0` = `4bd55fe`。之後同分支才改了 `state.rs`／`session.rs`／`types.rs`，所以對 `HEAD` 跑同一條
指令**不會**是空的；fixture 是否仍是「這個版本真的會寫出來的形狀」由測試
`character_session_fixtures_are_what_this_version_writes` 每次重生比對，不靠這條 diff。）

`hash` 只涵蓋 `state`，不涵蓋 `format`／`epoch`／`revision`，所以拿掉 `format` 不需要重算 hash。

## 這不是什麼

這些 fixture 只證明「**這一版**讀得懂那些檔案」。它們**不是**真機驗收，也不是
「v0.6.0 讀得懂現行格式」的證明——後者由
`crates/interaction-session/tests/session.rs` 的
`a_format_1_snapshot_is_still_readable_by_the_v0_6_0_snapshot_shape` 以一個複製自 v0.6.0
形狀的本地型別證明。
