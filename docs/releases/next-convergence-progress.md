# 下一輪工程收斂：差異矩陣與續接

## 1. 起點與授權

2026-09-06：起始 `main` / 遠端 main `622c8bf70963e343e51f92f5ecdf7fce4b37f516`，工作區乾淨，沒有其他 worktree。
施工分支 `feature/semantic-recovery-convergence`。使用者已授權完成實作、必要測試與獨立 Find → Verify 審查後，rebase merge main、確認實際 main SHA 的必要 CI，再建立並推送 annotated tag 及驗證自動 Release。不可跳過 gate 或移動既有 tag。

## 2. 本輪差異矩陣

| 需求 | 現有實作與 production 路徑 / owner | 契約 / 測試 | 差異與本輪動作 | 狀態 / 證據 |
|---|---|---|---|---|
| N1 SemanticState | session/state → snapshot/patch → TS SessionClient / Swift SessionClient | character-session、conformance；state_semantics / state_hash_fixtures / 三端 receiveDecisions | 生成語意 schema；consumer 驗證；保留未知數值原文；凍結發布樣本；optional 欄位演練 | 實作中；code-read + 起點 test-run |
| N1 既有 null 防線 | semantic_state_never_serializes_a_null、every_semantic_state_field_appears_in_at_least_one_state_hash_fixture | drills F4/F5；known-limitations §6.2 舊宣稱矛盾 | 重用現有測試、補保護範圍與修正文檔，不重做已存在機制 | verified-existing；起點 Rust 1245 項包含上述測試 |
| N2 同步回執 | DeviceLink / DeviceOutbound → note_full_state_delivered → derive_sync_profile → characterSync | device-profile / transport-bindings；declarative_session_loop / serial pty | write 成功不等於 applied；協商回執、綁定世代與狀態 tuple、有界 tracker、mobile 對稱接線 | 實作中；code-read |
| N3 未解決停止 | Runtime SensorSource → unresolvedStops → status / API / tray | privacy §5.1；sensors_loop | 摘除 / stop 前 journal、跨重啟 unknown、同 ID 不冒證、overflow/storage health | 實作中；code-read，新回歸待跑 |
| N3 裝置選擇 | declarative_lifecycle / registry；keptDisabledReceptors | device-profile；declarative_session_loop | 核實自然斷線關閉選擇；外部動器皆 requires_consent，維持 rebind default-off | 實作中；code-read；不把安全預設當 bug |
| N4 設定恢復 | CompanionPage → prefs patch + Runtime proactive configure；marker 在 desktop.json | applyPresetPlan / companion-preset-recovery / src-tauri prefs tests | 將協調放 host 應用服務；config revision / op ID / conditional retry；真檔案 crash/restart | 實作中；code-read + 起點 Tauri 63 / TS 1816 |
| N5 擴充演練 | scripts/drills、角色 store、schema/codegen | MAINTAINERS-MAP / drills | 本機 wt/drills 演練轉成 clean checkout 腳本；移除走 application function 而非直接 rm | 實作中；code-read |
| N6 一般模式 | 五入口、CompanionPage、AX walkthrough / Playwright | general-mode-ux / general-mode-tasks | 同環境任務與恢復走查、390px / a11y；真機與真人另列 | 待執行；歷史證據不冒充本輪 |

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

目前checkpoint `a32258ea161df537f2ad6c1f25cd70d86910d0da`，主要產品實作在`30ff852f5d893215c019d982774c8b59ab010659`；後續review/harness/Browser修正尚未提交。沒有使用者無關工作被移除。

1. N3 raw-capture write-ahead回歸已獨立1/0＋scope5/0。最新原生手機10步completed，含per-device unknown直接移除及程序重啟保留；UI成功通知漏看historical unknown的獨立finding已修，sharedprojection/三入口已統一；作者111/0、root獨立9/0，Browser sensors3/0。最新nativebuild已完成（87.75s）。
2. 整合Rust1278/0、Tauri78/0、TS1916/0、iOS163/0；CLI96/0。XCTest gate新增精確163計數與最後套件檢查，14/0独立回驗。architecture Rust233/0、TS230/0、Swift58/0；彙整變數界線bug已修，需整套runner再跑。
3. FullBrowser first attempt80pass5fail7notrun；4個legacy手機期望已改成更嚴格的未協商/未確認/無綠勾＋實際diagnostics，8支targeted全過（41.5s）。sensor fresh1/0、舊sequence1pass1fail1notrun，修teardown使同連線先回false再離開，原期待不改sequence3/0。尚待全套重跑。
4. 最新原生preset10/0（108.17s）、backup8步（4completed/4correctly-blocked；追加唯一下載nonce與來源pin後仍需最終重跑）、mobile10/0（334.907s）、work4/0已走通。所有手機/AI為fixture，非真機/真AI；無consent授予/人工解除。現在以相同新版AX driver重跑v0.7 baseline clean/legacy，之後candidate clean/legacy及後三native tasks各兩種設定。
5. 中途證據與Find/Verify原始logs已部分封存`docs/releases/evidence/2026-09-06-convergence/`；正式草稿`v0.8.0-final-report.md`、`v0.8.0-drills.md`未標完成。五項clean HEAD drills、暖機＋交錯3次perf、版本prepare、exactcandidate/main CI、verify/tag/Release資產安裝仍為下一階段。

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
