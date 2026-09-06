# 2026-09-06 收斂證據

`baseline-*` 是起点 `622c8bf70963e343e51f92f5ecdf7fce4b37f516` 實跑。其餘 initial-log-index 所列 integration log 是未提交工作樹的中途測試；log 中 sha 是當時 HEAD，**不是該 SHA 的已提交 source**。正式 checkpoint、release candidate 與 main 結果會分別索引，不能互相替代。模擬器與真 Tauri 分列，沒有真機或真人成果。

## 最終工程 checkpoint

`3b375bde4188e4a039852881d48d82ea9b619ce1` 的乾淨架構/五演練與原生clean/legacy9runs，Browser/TS最後產品修正source`87b0d031`，已歸檔在[final manifest](final/artifact-manifest.json)。Native App binary SHA-256 `1103cda7f8491ece2adbaab478056d14a174da47ab8536a2d652c734a0765f19`；build source87b0d031，3b375bd只改驗收gate/證據/文件，raw JSON仍各自保存實際來源。

[Find→Verify最後映射](final/review-closeout/convergence-find-verify-final-map.md)包含30列，其中3列是需求新增回歸、不捏造為独立finding；必要獨立驗證缺口0，product/harness open0。N1在未修改原版的獨立驗證另有probes/source hash與red/green。最終map保留作者與Finder/Verifier身份差異；歷史source/archived digests不混用。

final下的.log改為.txt，JSON/Markdown的已知artifact鏈結改為repo可讀路徑；完整manifest另記原始與歸檔SHA-256。嵌入的舊來源hash仍指原始bytes；不能用它核對經鏈結正規化的JSON。中途失敗與修正前輸出仍保留，沒有用新結果覆蓋舊紅燈。模擬器/pty/真Tauri/AI圖像檢視/真人未測分列；效能與發布source/CI/tag/資產的最終紀錄接續補入，這份工程manifest不等於Release成功。

final文字檔另外正規化行尾空白與多餘末行；原始hash仍保留。`.diff`證據使用JSON容器的`content`字串無損保存原始patch（取出即可核對originalSha256），不刪diff context空白而使patch失效。
