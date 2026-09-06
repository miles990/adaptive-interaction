# 下一輪工程收斂：差異矩陣與續接

## 1. 起點與授權

2026-09-06：起始 `main` / 遠端 main `622c8bf70963e343e51f92f5ecdf7fce4b37f516`，工作區乾淨，沒有其他 worktree。
施工分支 `feature/semantic-recovery-convergence`。使用者已授權完成實作、必要測試與獨立 Find → Verify 審查後，rebase merge main、確認實際 main SHA 的必要 CI，再建立並推送 annotated tag 及驗證自動 Release。不可跳過 gate 或移動既有 tag。

## 2. 本輪差異矩陣

完整 owner／production path／契約／保護與限制見 [final report §2](v0.8.0-final-report.md#2-需求差異與-production-呼叫路徑)。N1契約/codegen/consumer驗證、N2有界applied、N3持久unknown與UI三入口、N4host設定恢復、N5五演練及N6原生工程子集均已完成本輪實跑。
既有null序列化與hash欄位覆蓋、非consent受器keptDisabled仍分列verified-existing；Removed/無consumer ports為experimental。人工解除與真機/真人驗收是needs-environment/not-run，不冒充fixed。

## 3. 本輪基線（起點 SHA；本機 macOS / Apple Silicon）

命令與原始 log 暫存 `/tmp/adaptive-convergence-20260906/`，整合時將必要證據納入既有 evidence index，禁止只有本機 scratchpad 的交付。
Rust 1.94.0、Cargo 1.94.0、Node 24.5.0、pnpm 10.27.0、Xcode 26.6。建置設定 `CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4`，mobile advertise 關閉。

| 命令 | 結果 | 牆鐘時間 | 證據等級 |
|---|---|---|---|
| cargo test --workspace --no-fail-fast | 1245 passed / 0 failed / 0 ignored，96 test binaries（含 doc-tests 的一致加總） | 308.04 s | 本輪 test-run；fixture / pty 均為模擬器 |
| cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml | 63 passed / 0 failed | 57.41 s | 本輪單元，非真視窗 |
| apps/interaction-desktop: pnpm test | 1816 passed / 0 failed，84 files | 23.87 s | 本輪 jsdom / fixtures |
| apps/interaction-desktop: pnpm build | typecheck + Vite build 成功 | 5.89 s | 本輪 build |
| iOS README simctl 注入 XCTest（專用暫存 simulator；執行後已刪除自身 simulator） | 158 tests / 0 failures；Xcode build + iOS 26.2 iPhone 17 simulator | 60.86 s（含建置/開機） | 模擬器，非真 iPhone |

## 4. 工作所有權

- semantic_contract：N1/schema/codegen/raw JSON/TS、Swift validated consumer、optional-state drill；Tauri raw IPC 局部。
- state_receipts：N2/device wire/runtime state delivery/mobile receipt/characterSync/serial simulator。
- restart_lifecycle：N3/journal/lifecycle/sensor health/API、CLI、tray 的停止投影。
- 整合者：N4/prefs 與設定 use case、N5其餘演練、N6驗收、發布與所有進度/CHANGELOG/acceptance/evidence index。

## 5. 下一動作

已提交驗收checkpoint `3b375bde4188e4a039852881d48d82ea9b619ce1`；產品最後變更 `87b0d03173cdefce68bad2403b7f76be59b738e1`（歷史unknown不發整體成功通知）。主要實作在`30ff852f5d893215c019d982774c8b59ab010659`。目前只整理本輪證據/文件，無使用者無關修改被清理。

1. 本輪完整Browser92/0（222.34s），TS1916/0；乾淨checkpoint architecture六組全過（190.36s）：docs172/0、release-scripts58/0、TS230/0、Rust233/0、nativeSwift58/0，四runner覆蓋五演練。原始log不重複加總。
2. 同一release App SHA-256 `1103cda7f8491ece2adbaab478056d14a174da47ab8536a2d652c734a0765f19`：settings clean/legacy各8步（4completed/4correctly-blocked）；work各4completed；mobile各10completed；preset10completed；basic各9completed＋1人工解除needs-environment，全部9runs exit0。CLI/fixture/driver hash固定，埠已清理。詳見[一般模式](v0.8.0-general-mode-tasks.md)。
3. N1兩個原始缺陷已由非作者在未修改baseline622c8bf獨立反駁重現並回綠：TS2controls/5fail→7/0，Rust2controls/1fail→3/0。完整Find/Verify整理保留角色與歷史紅燈，不拿作者執行冒充獨立執行。
4. [效能比較](v0.8.0-performance.md)完成：source622c8bf vs3b375bd，每版一暖機＋交錯三次sample，8runs全部成功；未越過事前p95/有界性/GC成長調查預算。不是全產品無退步或長期無leak宣稱。原始數據、完整source/輸入digest已封存repo。
5. 工程與效能gate已完成；`release-prepare.sh 0.8.0`已成功（40.51s），四份manifest/兩鎖檔/OpenAPI版本已核對，frozen corpus與TS/Swift生成內容沒有被改動。現在提交正式release candidate並跑完整驗證，再建立PR；minor0.8.0理由已依實際變更寫在migration。後續為正式candidate完整驗證→push/PR/必要CI→rebase merge→實際main必要CI→同SHA完整release-verify→annotatedtag→Release資產下載/checksum/可用macOS安裝smoke。尚未執行的步驟不填預測SHA或run ID。

## 6. Blockers

真iPhone、ESP32與非開發者受測者目前沒有提供；真機/真人欄維持needs-environment/not-run。這些是既有發布政策已保留的明示限制；新功能不得因此宣稱真機已驗收，仍須完成既定工程gate。
Claude Code Workflow runtime無直接callable工具；獨立Finder＋另一Verifier使用既有agent工具執行相同Find→嘗試反駁→具體確認流程。審查角色與工具差異會保存在正式review report。

## 7. 中途證據（未提交工作樹，不能當成正式candidate SHA驗收）

- Baseline Rust1245/0、Tauri63/0、TS1816/0、iOS158/0；原始SHA與環境見§3。
- N1 corpus44states＋4patches；native Swift58/0、TScontract/transport59/0、Rustcontract5/0。五項獨立attention/date probes三端修後一致。
- N2 Mobile36/0、adapter58/0、UI138/0；Serial完整套用/patch/拔插/新世代case通過；default/BLE韌體compile通過（非真板）。
- N3 review5/0、sensors38/0、mobile81/0；新增直接mobile移除回歸parent獨立0/1紅後已修，完整workspace1278/0。
- N4 native中途build恢復matrix10/0；最新Tauri全套78/0、TS全套1907/0。新的backup driver驗真正下載與Open picker，尚未finalrun。partial的component回覆遺失與真proxy fault證據分列。
- N5 restricted五命令、lifecycle四步、package真hostport1/0＋UIfixture2/0、optional-state21步均中途通過，待乾淨checkpoint再跑。
- N6兩個v0.7.0真AX baseline各9completed＋1needs-environment；人类耗時/求助未量。Screenshot工具在這環境未產出圖，AX不冒充像素review；390px Chromium另驗。對话框Browser red0/1→green1/0，shared元件71/0。
