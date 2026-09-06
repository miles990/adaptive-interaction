# v0.8.0 發布階段原始證據

來源與完整交付見 [publication](../../../v0.8.0-publication.md)。`artifact-manifest.json` 分開保存原始檔與封存檔的SHA-256，並列出文字正規化。`.py.txt`／`drivers/cargo.py.txt` 是當次驗證harness的來源快照，不是產品功能或穩定CLI；既有可執行安裝器／原生runner仍由tag中的scripts擁有。

階段來源不可混用：

- candidate原始測試在clean c15873ac；PR5 rebase後main為1fa69b8，兩者tree一致。
- main CI、完整本機verify及tag對1fa69b8驗證；第一次workspace非0仍保留為not-reproduced。
- tag helper自查跳過full matrix；完整矩陣是其前同SHA的0skip驗證。
- 安裝App來源是Release tag；AX driver在clean 4f599293執行，該commit只多進度文件，driver/helper/fixture byte與tag一致。
- publication文件檢查在4f599293之上的尚未提交文件執行，之後由文件提交封存；log的parent HEAD不代表那些文件改動已存在於parent。文件提交不冒充被tag的產品來源。
- 獨立文件審查首次核對204個raw artifacts；closeout追加其本身報告與檢查結果，不把追加檔案的數量倒填為原審查範圍。

CLI安裝、DMG首次別名檢查失敗及修復、native原始步驟、CI與Release全log分別保存。runtime home、token與大型安裝binary不入版控；未跑或環境缺少不計通過。路徑中的`/tmp`若未轉成相對連結，是原始執行環境的provenance，不表示驗收檔案仍只能在scratch取得。
