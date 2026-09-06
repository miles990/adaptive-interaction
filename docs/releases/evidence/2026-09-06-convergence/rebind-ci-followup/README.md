# 發布後 rebind 測試競態證據

說明見[CI 後記](../../../v0.8.0-ci-followup.md)。原 CI 失敗與受控原紅、嚴格讀取原紅、修後綠及獨立 Find → Verify 分開保存；完整 Rust 在修正 commit `857f009acf0b2c217f12ec041766ad7de2a2c7cf` 執行。

`artifact-manifest.json` 綁定原始與封存 SHA-256。文字 log 只正規化尾端空白並以 `.txt` 保存；JSON 已知 artifact 路徑相對於本目錄。`.diff.json` 是保存完整 patch 字串的 JSON，需先解碼 `patch`，不能直接當 patch 套用。

受控測試 binary、大型 runtime 原檔及暫存 home 不納入版控；binary hashes、完整命令與 source commit／probe diff 已保存。這些是 750ms 測試時序注入及 pty 模擬器，並非發布 binary 或真板。產品 probe 已還原；v0.8.0 tag／資產未改。

最後補入文件獨立審查與文件gate原紅／修後綠3份，manifest合計50份。文件Finder核對的是先前47份核心證據；不把後補資料冒充先前已審查。文件gate第一輪因Unreleased缺少Known limitations小節184/1，補齊後188/0，release-scripts58/0；非產品測試失敗。
