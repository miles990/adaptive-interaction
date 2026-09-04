# Changelog

本專案採 [Keep a Changelog](https://keepachangelog.com/zh-TW/1.1.0/) 格式與
[語意化版本](https://semver.org/lang/zh-TW/)（`MAJOR.MINOR.PATCH`）。

版本一致性：workspace `Cargo.toml`、`apps/interaction-desktop/src-tauri/Cargo.toml`、
`apps/interaction-desktop/src-tauri/tauri.conf.json`、`apps/interaction-desktop/package.json`
四處版本必須相同——用 `scripts/release.sh <version>` 一次搞定。

## [Unreleased]

### Fixed
- `release.yml` 桌面 `.sha256` 上傳迴圈在 Windows 上讀到 `\r\n` 清單會把 `\r` 留在檔名尾巴、`gh release upload` 找不到檔案
  （v0.6.0 Release run 33918252926 的 `Desktop (windows-latest)` job 因此紅、`finalize` 依設計 skipped）。現在逐行去掉 `\r`，
  python 端也固定用 `newline="\n"` 寫檔；`scripts/tests/release-scripts.sh` 以假 `gh` 重現 CRLF 清單釘住（44/0）。
  v0.6.0 的兩個 Windows `.sha256` 由整合者從已上傳資產計算並上傳、本機重跑 finalize 盤點後手動 publish；tag 未移動
  （詳見 `docs/releases/v0.6.0-final-report.md` §34）。

### 保護行為核對（沿用 0.6.0，本段未改動）
- 桌面角色視窗的主機端點擊穿透輪詢 80ms（`CLICKTHROUGH_POLL_MS`）不變。
- Runtime Session Host（`character_session.rs`）、桌面同步卡（`CharacterSyncCard.tsx`）、iOS Session client
  （`SessionClient.swift`）皆已於 0.6.0 落地，本段只修發布 workflow，不動它們。

## [0.6.0] - 2026-09-05

### v0.6.0 — Foundation：AIP 1.0 最小協定、權威 Character Session、小樞脫離核心

> 保守、可回退的架構升級：不重寫既有功能，先建立修改前基線與恢復矩陣，再以 Strangler／feature flag
> 逐條路徑替換。本段只記錄**已提交**的事實，證據等級逐項標明；未落地的項目列在文末「已知限制／尚未落地」。
> 通過數字一律引自 `docs/releases/v0.6.0-{baseline,test-matrix}.md`，本檔不做第一手宣稱。

#### Added — 協定與領域核心
- **AIP 1.0（Adaptive Interaction Protocol）**：新 crate `interaction-aip`（純函式、無 tokio／I/O）——
  versioned envelope（`aip/1.0`）、十二種 message type 與各自的 profile 必填規則、十二值 Outcome 誠實階梯
  （received≠accepted≠applied≠observed≠claimed-completed≠verified）、19 個穩定錯誤碼、版本協商（同 major、
  min minor、`newerMinor`）、確定性能力協商（交集＋min）、身分綁定決策（宣稱不符一律拒絕，不「修正後執行」）、
  有界去重環、離線事件政策表、證據分類、canonical JSON hash 與上限（訊息 64 KiB／payload 32 KiB／深度 8／
  字串 2000）。未知 message type 會解析成 `Unknown` 但**永不執行**；未知選填欄位 round-trip 不遺失；錯誤訊息
  不回顯輸入。契約：`docs/aip/README.md`。
- **Schema 單一來源**：`schemas/aip-1.0.schema.json` 由 Rust 型別產生（golden，`GOLDEN_UPDATE=1` 重生），
  `scripts/aip-codegen.mjs` 確定性產生 `apps/interaction-desktop/src/aip/generated.ts` 與
  `apps/interaction-ios/.../AIPGenerated.swift`（含內嵌 fixtures），`pnpm aip:check` 在漂移時 exit 1 並納入 CI。
  38 份 golden fixture 由 Rust（10 conformance）、TypeScript（91 vitest）、Swift（14 XCTest，iPhone 17 **模擬器**）
  三方共用；`tests/e2e/tests/dependency_boundaries.rs` 釘住純 crate 不含 tokio／axum／tauri／transport 依賴。
- **權威 Character Session**：新 crate `interaction-session`（純函式）——語意狀態（mood／activity／attention／
  truth／lastInteraction／members）與唯一 owner、確定性 Director（touch→happy／playful 反應 intent；
  `task.verified`→proud＋celebrate；emergency→frozen 且拒絕互動）、單調 revision／sequence、RFC 7396 patch＋
  SHA-256 state hash、有界事件日誌（512）delta replay／snapshot fallback、epoch-aware resume、每成員去重環、
  deadline 過期、rate limit、membership／presence、十三關固定順序的安全管線、CPP 投影（celebrate 不投影到桌面，
  避免與既有 `verified-success` 雙播）、ports（Clock／SessionStore／IdentityVerifier／ConsentVerifier／
  RendererPort／DevicePort）與不含 secret 的 diagnostics。70 個測試含安全矩陣。契約：`docs/aip/character-session.md`
  （含 State Ownership 表）。

#### Changed — 小樞脫離協定核心（Strangler，行為不變）
- `crates/interaction-character`（CPP 核心）不再含任何小樞字串：`SHU_RIG_VARIANTS`／`shu_rig_capabilities()`／
  rig-pack 遷移搬到新 crate `interaction-character-shu`（`ShuRigPack`、`RigPackMigrator`）；核心新增
  `PackMigrator` trait 與有界 `MigrationRegistry`（sprite 遷移留在核心）；`ValidationLimits::default().builtin_whitelist`
  改為**空**，host 必須注入（Runtime `character_host_registry()`：shu-rig／shape／sprite／text，Tauri 匯入沿用同一份；
  `BUNDLED_CHARACTER_IDS` 改由 `public/characters/index.json` 解析）。`migrate_legacy_pack` 標 `#[deprecated]`。
- 桌面角色視窗的點擊穿透行為不變：仍是多框 hit regions、fail-closed、主機端點擊穿透輪詢 80ms（`CLICKTHROUGH_POLL_MS`）；本輪只動 adapter 建立方式，不動 host 攔截。
- 桌面 TS：`character/adapterRegistry.ts` 取代 `CompanionApp`／`gatewayWiring` 的 entrypoint if-chain，shu／sprite／
  text 各自註冊並以 meta（palette／playfield／surface）表達差異；`architecture-no-entrypoint-switch.test.ts` 讀原始碼
  鎖住不再有字面分岔；`adapter-contract.test.ts` 對四個內建 adapter 跑同一套生命週期／unsupported／cancel／dispose／
  資源清理契約。
- **第二個 Reference Character `ref-shape`**（幾何形，只有 visual.presence／visual.expression／input.click，無耳尾、
  無音效、無玩具）：加入它只動 manifest、adapter、兩個 host 白名單清單、測試與文件（`docs/aip/reference-character.md` §5），
  核心 `interaction-character/src`、`character.rs`、`CompanionApp.tsx` 沒有為它新增任何分岔。

#### Changed — 發布流程
- `scripts/release-verify.sh` 的 secret 掃描擴充到 GitHub（`gh[pousr]_`／`github_pat_`）、Anthropic／OpenAI（`sk-ant-`／`sk-proj-`／`sk-`）、AWS（`AKIA`）、Google（`AIza`）、Slack（`xox[abprs]-`）與私鑰形狀；`scripts/tests/release-scripts.sh` 以 9 個正例＋4 個反例釘住（對抗審查 `release-provenance-076`，該項在報告中為 unverified，順手修正、不計入 confirmed 統計）。
- `scripts/release.sh` 拆成 `release-prepare.sh`（改版本號／CHANGELOG／golden／codegen，不 commit、空的 Unreleased
  直接拒絕）、`release-verify.sh`（唯讀關卡：worktree 乾淨、四處版本一致、CHANGELOG 段非空、tag 不存在、
  `openapi.json` 版本、tracked 檔無 secret、AIP codegen 無漂移、HEAD 的 CI check-runs 全綠、可選全量測試）與
  `release-tag.sh`（只從已在 origin 上且 verify 通過的 commit 建 annotated tag，`--push` 才推）。`release.sh` 只印流程，
  保留 `--all-in-one` 給離線緊急情況。

#### Added — wave 2／3：Session Host、iPhone 閉環與桌面同步（證據等級逐項標明）
- **Runtime Session Host**（`crates/interaction-runtime/src/character_session.rs`，`e71ab45`）：把
  `interaction-session` 的權威 Session 接進 Runtime，並綁到 iPhone 線協定、HTTP、SSE 與 CLI。
  由 feature flag `INTERACT_AI_CHARACTER_SESSION`（預設 `1`）控制；`0` 時四條
  `/v1/character-session/*` 路由回 `503 session-disabled`，行為回到 v0.5.1。
  **證據等級：unit／contract**（`crates/interaction-runtime/tests/character_session_loop.rs`）。
- **iPhone 線協定 v1 的 `aip` frame**（`crates/interaction-runtime/src/mobile.rs`，`e71ab45`）：
  只在 `auth-ok` 之後接受，共用既有 128 KiB frame 上限與 30 msg/s 速率窗；沒送過 `capability`
  的舊 App 永遠收不到任何 `aip` frame。**證據等級：fixture**（模擬 iPhone，非真機）。
- **桌面同步卡**（`apps/interaction-desktop/src/components/CharacterSyncCard.tsx`＋
  `src/statusProjection.ts` 的 `CHARACTER_SYNC_PROJECTION`，`f28bb84`／`6683403`）：角色頁新增「同步」卡，
  以一般模式人話呈現 session 狀態（不外洩 envelope／revision／capability 等技術詞），
  安全訊息仍走可信 host overlay。**證據等級：browser fixture**（vitest＋Playwright，非真機 iPhone）。
- **iOS Session Client**（`apps/interaction-ios/InteractionCompanion/Services/SessionClient.swift`，`012ff69`）：
  iPhone App 加入 session 並由 semantic state 驅動角色呈現。
  **證據等級：simulator**（iOS Simulator XCTest，`SessionClientTests.swift`；iPhone 真機未驗）。
- **v0.6.0 對抗審查已執行**：`.claude/workflows/adversarial-review-v06.js` 的 findings 落盤於
  `docs/reviews/adversarial/6683403-20260904T161327Z.{json,md}`（`5689b4f`）。

#### Fixed — 發布來源可信度與文件誠實度（本輪對抗審查 confirmed findings）
- **`self update`／`install.sh` 的 sha256 驗證改為 fail-closed**（`release-provenance-074`）：
  抓不到 `<asset>.sha256` 不再印一行 warning 就照裝，而是中止安裝。要接受未驗證下載必須明示
  `INTERACT_AI_ALLOW_UNVERIFIED_DOWNLOAD=1`。**Breaking**：Release 資產尚未上傳完成時安裝會失敗
  （這是刻意的——draft release 期間本來就不該被安裝）。
- **桌面安裝包有 checksum 了**（`release-provenance-075`）：`release.yml` 的 `desktop` job 為每個
  bundle 產生並上傳 `.sha256`；`interact-ai self install-desktop` 下載後先驗證才交給 OS，驗證失敗
  會刪掉下載檔並中止。
- **tag → Release 有 CI 關卡，且先 draft 後發布**（`release-provenance-072`／`079`）：
  `release.yml` 新增 `ci-gate` job，用 `scripts/ci-required-checks.sh` 斷言 `ci.yml` 定義的每個 job
  在被 tag 的 commit 上都存在且 `success`（`ci.yml` 不在 tag 上跑，`--verify-tag` 只證明 tag 存在）；
  `create-release` 改以 `--draft` 建立，新增 `finalize` job 在四個平台的 CLI、桌面 bundle 與 extras
  全部到齊後才 `--draft=false`。任一平台建置失敗，Release 就停在 draft，安裝器看不到它。
- **`release-verify.sh` 不再把「沒查」寫成「通過」**（`release-provenance-073`）：跳過的關卡印
  `⊘ … SKIPPED`，收尾改成 `passed-with-skips`；CI 關卡改為逐一斷言必需 job 在場且成功
  （`release-provenance-077`，含 `gh api --paginate`）；新增「每個 crate 版本都跟著 workspace」關卡
  （`release-provenance-078`）與文件誠實度 lint 關卡。
- **`release-tag.sh` 在 macOS 預設 bash 3.2 下不再 crash**（`release-provenance-071`）：`set -u` 下的
  空陣列展開改用 `${ARR[@]+"${ARR[@]}"}`。修復前「不跳過 CI」的安全路徑會 unbound variable 直接退出，
  只有 `--skip-ci` 能跑——關卡被 shell 相容性反向篩選。
- **`release.sh --all-in-one` 需要明示 `--i-know-there-is-no-ci`**（`release-provenance-072`），
  且 tag message 會寫入「無 CI 證據」。
- **`interaction-agent-gateway` 版本跟著 workspace 走**（`release-provenance-078`）：修復前它固定是
  `0.3.0`，並經由 `clientInfo.version` 洩漏給外部 Codex agent。
- **Linux aarch64 不再被謊稱支援**（`release-provenance-080`）：`release.yml` 從未建置該 target，
  `get.sh` 與 `target_triple()` 改為明確指向從原始碼建置，而不是讓使用者拿到 HTTP 404。
- **文件與程式碼現況對齊**（`evidence-honesty-012`／`013`／`014`／`016`）：CHANGELOG 不再宣稱已落地的
  wave 2／3 功能「尚未落地」；`docs/ARCHITECTURE.md` §6 改為 HEAD 現況；`docs/aip/threat-model.md`
  改用函式名／步驟錨點取代會漂移的硬編行號；`docs/aip/README.md` §10 改為誠實標示 `EvidenceClass`
  尚未接進 diagnostics。新增 `scripts/tests/docs-claims.sh` 把這些宣稱釘成 lint。

#### 對抗審查 `6683403-20260904T161327Z`（80 送審／73 confirmed／已修 68／部分修 5／deferred 0）

12 維度、find→獨立 verifier 覆核；7 則 refuted、1 則 unverified 不計入 confirmed。逐條處置在
`docs/releases/v0.6.0-known-limitations.md` §2.1，完整報告在 `docs/reviews/adversarial/6683403-20260904T161327Z.{json,md}`。
修復依檔案歸屬分成 7 組平行進行，每組先寫「舊行為下紅燈」的回歸測試再修。

- **已修 68**（根因消除＋回歸測試）。其中 6 則需要跨組收尾（一組改不到另一組的檔案），
  由最後一輪收尾補齊：`pairing-migration-001`（Rust 端 `accept_state_with_epoch` 改成
  「`session-reset` 且 epoch **不同**即接受」，與 iOS 端與契約 §7 第 4 步一致；含 host 重灌後
  epoch 變小的回歸）、`session-integrity-056`（Runtime 啟動時若 estop 仍生效就補送
  `RuntimeFact::Emergency{engaged:true}` 進 session，且排在 `restore_agent_sessions` 之前）、
  `identity-binding-009`（`character_session_device_query` 對去重命中直接回
  `accepted{duplicate:true}`，不重跑 resume／snapshot、不消耗 sequence）、
  `general-mode-ux-022`／`capability-consent-052`（`MemberView`／diagnostics `members[]` 新增
  `unsupportedIntents: [String]`，讓一般模式的「部分能力目前不可用」有真實來源）、
  `reconnect-recovery-044`（`mobile.rs` 斷線收尾對已協商成員送 `Presence::Reconnecting`，
  逾時後才由 session tick 轉 `Offline`；撤銷仍是 `leave`）。
- **部分修 5**（主要缺口關掉、剩餘範圍逐條記在下方「已知限制」）：`identity-binding-007`、
  `runtime-boundaries-065`、`release-provenance-078`、`character-package-020`、`aip-protocol-036`。
- **deferred 0**：沒有任何 confirmed finding 被留成「知道但沒動也沒記錄」。

#### 已知限制（v0.6.0 進行中，尚未修）
- **`runtime-boundaries-065`：高風險受器的停止路徑只做到誠實回報，沒有做 `SensorSource` port**。
  stop-all-sensors 對非 mobile provider 的高風險受器會回 `stopped=false` 與
  `SensorStopUncertain{no-stop-path}`（不再假裝停掉了），但那些受器**仍然收不到真正的停止請求**。
  結構性修復（把「停止感測」抽成 provider 都要實作的 port）不在本輪範圍。
  測試：`crates/interaction-runtime/tests/sensors_loop.rs::stop_all_sensors_is_honest_about_high_risk_receptors_it_never_asked`。
- **`settingsTransfer.ts` 仍以 `SHU_RIG_PALETTES` 驗證使魔配色**：小樞脫離協定核心之後，桌面設定
  匯入／匯出這條路徑還留著一個對小樞 palette 表的引用（`isShuRigPalette` 的相容 re-export）。
  行為正確，但它是「Runtime／頁面不得再引用小樞」這條不變量在前端的最後一個例外。
- **burst > 30 msg/s 會觸發既有 v1 連線關閉**：AIP frame 共用 iPhone 線協定 v1 的速率窗，
  超過就是**關連線**（不是只丟那一則）。session 端另有每成員 30/s 的 token bucket 會先回
  `rejected{rate-limited}`，但一次爆量仍可能先撞到 transport 那一層。這是既有 v1 行為，本輪不改。
- **iPhone 真機仍是 implemented-unverified**：wave 1-3 與本輪修復的 iOS 證據全部來自
  iOS Simulator（XCTest）與程序內 fixture（模擬 iPhone）。`docs/releases/v0.6.0-test-matrix.md`
  記載「iPhone 真機／ESP32 真板：未執行」，本檔沿用同一結論。
- **沒有程式碼簽章／公證／SBOM／build provenance**：Release 只發布 `.sha256`，它證明「位元組與 Release
  一致」，不證明來源。macOS bundle 未簽章未公證、Windows 安裝程式未簽章（`release-provenance-075`
  的 provenance 部分只做到誠實揭露，見 `docs/INSTALL.md`）。
- **Linux aarch64 沒有預編譯檔**：需從原始碼建置（`release-provenance-080`）。
- **兩個 crate 的版本仍脫離 workspace**：`crates/interaction-adapter-declarative`（`0.2.0`）與
  `adapters/media`（`0.2.0`）寫死自有版本，`release-prepare.sh` 改不到它們。`release-verify.sh`
  會以 `⚠ 已知版本漂移` 明列，不當成通過（`release-provenance-078` 的剩餘部分）。
- **`EvidenceClass` 只有詞彙、沒有機制**：diagnostics 回傳沒有證據等級欄位，「fixture 不得標成
  real-device」目前靠人工文件紀律（`evidence-honesty-016`）。
- **`ci-gate`／`finalize` 未經真實 tag push 驗證**：兩個新 job 的邏輯由
  `scripts/tests/release-yml-embedded.py` 抽出內嵌 shell／python 實跑（資產不齊會擋、缺 check-run 會擋），
  但整條 workflow 只有下一次真的推 tag 時才會被 GitHub 執行。**證據等級：unit，不是端對端。**

#### Added — 基線與審查
- `docs/releases/v0.6.0-baseline.md`（修改前同機實跑：Rust 827/0、Tauri 50/0、vitest 1168/0、CLI E2E 82/0、Playwright 65/0、
  ESP32 兩組態、iOS typecheck＋XCTest 46/0 模擬器；daemon 與角色效能基線）與 `docs/releases/v0.6.0-recovery-matrix.md`
  （九列恢復矩陣、受保護行為與「無測試鎖住」清單、未盤點模組）。
- `.claude/workflows/adversarial-review-v06.js`：12 維度 v0.6.0 對抗審查（尚未執行）。
- iOS README 補記 XCTest 注入需要 `lib_TestingInterop.dylib`（v0.5.1 文件漏寫，基線階段實際踩到）。

#### 回歸（對抗審查修復後，最終整合回歸；同機實跑，2026-09-05）

> 逐項數字、命令與各 wave 的疊加過程見 `docs/releases/v0.6.0-test-matrix.md` §8「最終回歸（對抗審查修復後）」。

Rust workspace **1040 passed / 0 failed（85 個 test target）**（基線 827／66 → **+213／+19**）、Tauri backend
**56 passed / 0 failed（3 target）**（基線 50 → **+6**）、前端 vitest **1416 passed / 0 failed（70 檔）**
（基線 1168／60 → **+248／+10 檔**）；`cargo fmt --check`／`cargo clippy --workspace -D warnings`／
`aip:check`／`pnpm typecheck`／`pnpm build`／`git diff --check` 全部乾淨。

CLI E2E 第一次全套跑出 **95 passed / 1 failed**——失敗是驗收腳本本身的斷言寫錯（斷線後預期立刻變成
`presence offline`，但 `reconnect-recovery-044` 修復後的正確行為是先進 `Presence::Reconnecting`、逾時
才轉 `Offline`），修正斷言後重跑 **96 passed / 0 failed**（基線 82 → **+14**），非產品回歸。

Playwright 第一次全套跑出 **65 passed / 1 failed / 5 did not run**——`character-session.spec.ts` 的旅程與
CLI E2E 同一根因：斷線後仍等 `offline`／「iPhone 暫時離線」，而正確行為是先 `reconnecting`／「iPhone 正在
重新連線」；改測試期待（截圖改名為 `*-reconnecting.png`）後受影響 3 個 spec 重跑 **30 passed / 0 failed**，
隨後重新跑一次完整套件確認 **71 passed / 0 failed**（2.3 分鐘，基線 65 → **+6**），非產品回歸。

iOS `InteractionCompanionTests`（iPhone 17 模擬器）**101 passed / 0 failed**（基線 46 → **+55**，含新增
`SessionClientTests` 34、`AIPConformanceTests` 17（基線 14 → +3）、`ProtocolTests` 21（基線 17 → +4）、
`MotionClassifierTests` 8、`ReconnectHintTests` 21 不變）。

角色效能（`pnpm perf`，headless Chromium，非 Tauri WKWebView；量測時與其他對抗審查 agent 並行，非獨占機器）：
touch result 延遲 p50 2.6 ms／p95 4.8 ms／max 5.8 ms（n=20，全數 applied）、join→協商 2.0 ms、
reconnect→resume 3.3 ms、snapshot 862 bytes、patch 中位數 492 bytes、閒置 RSS 41.0 MB、活動 RSS 45.6 MB。
burst100（連續灌 100 則 touch）量測到前 30 則被接受並套用後，既有 iPhone 線協定 v1 的 30 msg/s 速率窗
把連線關閉、其餘 70 則未送達——是既有速率限制與量測腳本互動的結果，非本輪缺陷（見「已知限制」）。

## [0.5.1] - 2026-09-04

### v0.5.1 — 產品完成度、一般模式易用性、誠實狀態與剩餘技術債（修補版本，2026-09-04 發布）

> 以 v0.5.0（`8b713c7`）為基線的修補版本：修掉 v0.5.0 已知限制清單裡五個刻意保留的 partial finding
> （`ia-settings-012`／`safety-invariants-078`／`companion-gameplay-032`／`protocol-conformance-030`／
> `link-transports-054`）與直接影響一般模式主要流程的殘留缺口。每一項都附「舊行為下先紅燈」的 regression
> test；ESP32 仍是編譯＋模擬器、BLE 仍是 fixture、iPhone 真機本輪 **blocked**（鑰匙圈授權待人工，
> 見 `docs/releases/v0.5.1-iphone-device-evidence.md`）；本輪新增「真 Tauri 視窗」證據等級。發布關卡、測試矩陣、
> 已知限制與遷移指南見 `docs/releases/v0.5.1-*.md`。

#### Fixed — v0.5.0 刻意保留的 partial findings
- **`ia-settings-012`（精靈安靜時段預設封鎖桌面角色）**：首次設定精靈與角色頁改共用同一個 canonical
  quiet-hours builder（`src/quietHours.ts`：`QUIET_SILENCED_CHANNELS`＋`buildQuietHoursPatch`）。精靈不再送出
  `silencedChannels: []`（後端會把空陣列解讀成含 `desktop-pet` 的內建預設），一般安靜時段保留 L0 呈現，只有
  明確關閉角色顯示才隱藏。回歸：精靈 commit 斷言明確清單且不含 `desktop-pet`；`presentation_loop.rs` 以精靈新形狀
  建立安靜時段，斷言 L0 呈現不被擋、不進 Inbox，被靜音的 haptic 仍需人決定。
- **`safety-invariants-078`（agent interrupt 擁有權）**：`POST /v1/agent-sessions/{id}/interrupt` 不再對 legacy
  共享 agent token 放行——它沒有 session 身分、也不能建立或列出 session，無法證明擁有權，一律
  `403 token_scope_forbidden`；session-scoped capability token 只能中斷自己的 session（middleware 既有比對＋
  handler 第二層 `interrupt_principal_allowed`）；human token 保留管理能力；runtime 緊急停止走內部呼叫不受影響。
  **刻意的安全收斂**：以 `state/api-agent-token` 中斷 session 的 connector 需改用該 session 的
  `INTERACT_AI_SESSION_TOKEN` 或交給人類平面（skill `references/api.md` 已寫明遷移）。
- **`companion-gameplay-032`（角色舞台單一 hit-rect 聯集）**：Tauri host 改收多個 bounded hit regions
  （`companion_hit_regions` IPC：角色本體、每隻使魔、每個可抓玩具、真正互動的 UI 各一個矩形），游標只在落於
  某個 region 時才攔截，角色與遠處玩具之間的空白現在會點穿到桌面。防禦：region 數 ≤16、逐一 clamp 到視窗、
  單框不得兩軸皆 ≥80% 視窗、總面積 ≤80%、空清單／非有限值整份拒絕並沿用上一份有效 regions（fail-closed），
  host 端每 45 ms 最多接受一次；`companion_hit_rect` 保留為單矩形相容 shim；主機端點擊穿透輪詢 80ms 不變。
  renderer 端新增純函式模組 `src/companion/hitRegions.ts`（幾何、去重、與 Rust 同一組上限、回報政策
  ≥50 ms／位移 >4 px／≤60 ms 心跳），拖曳中角色 region 持續存在並外擴 8 px；快捷選單／氣泡／可信文字各自成
  region，整窗攔截只剩文字輸入與拖放確認。真 Tauri 視窗的點穿驗收見 `docs/releases/v0.5.1-*.md`。
- **`link-transports-054`（serial pty/file fallback 讀取執行緒洩漏）**：unix 上 fallback reader 改為另開獨立
  `O_NONBLOCK` fd，用 `poll(2)` 同時等裝置位元組與 supervisor 的 self-pipe 收攤訊號，shutdown 在
  `READER_JOIN_GRACE_MS` 內真的 join 回 reader，不再以 detach 作為正常結束；EAGAIN／EINTR 回到 poll，不 busy loop；
  真硬體 `serialport` 路徑（200 ms 逾時）與非 unix 平台不變；`detached_reader_threads()` 保留為警報器。
  回歸：沉默 pty 的 reader 在寬限期內回收；50 圈 open/shutdown soak（含與重連競速）零 detached、最差關閉 203 ms；
  reader 睡在 poll 裡（mutation 驗證）。`libc` 加為 unix 直接依賴（lock 內既有版本）。
- **`protocol-conformance-030`（配對驗證證據等級）**：DeviceLink 記下的「配對碼從未被比對」（裝置回報
  `hello.pairing=false`）現在從 driver 收據一路投影到 provider 證據：`ProviderTested` 新增 `pairingUnverified`
  （serde default，舊記錄不變），`tested_note` 改說「這次握手無法證明配對碼被比對過（裝置說它不需要配對），
  身分證據僅為裝置自報的 deviceId」（對抗審查 protocol-conformance-042 後的最終措辭），
  `GET /v1/providers/{id}` 帶出旗標，桌面六階階梯把「已測試」「已啟用」降為警告色的「（配對碼未驗證）」；
  不知道配對狀態的來源（人為測試、受器讀取）不會把先前的未驗證標記洗成乾淨。「裝置宣稱不需配對」不等於
  「配對已驗證」。

#### Fixed — 一般模式與誠實狀態
- **記憶匯出 `limitReached` 精確化（已知限制 #21）**：storage 新增 `count_memory(layer)`，`memory_export` 改以
  資料庫真實筆數判斷截斷（剛好 1,000 筆不再誤報），回應新增 `total`／`included: ["memory-items"]`，`notIncluded`
  精確列出 `knowledge-nodes`／`assets-and-derivatives`／`knowledge-receipts`／`character-interaction-memory`。
  匯出範圍維持只有記憶項目（方案 B）；「備份與還原」頁改為轉述後端回報的範圍，描述匯出的文案不再稱「備份」。
- **GlobalSearch 一般模式不再外洩記憶技術分層（已知限制 #23）**：搜尋結果改走共用的
  `memoryLayerLabel(layer, advanced, name)`，一般模式看不到 `agent-handoff`／`skill`／「Agent 交接」等字串。
- **角色生命週期人話集中（已知限制 #14）**：`CompanionPage.characterLiveState` 併入 `statusProjection.ts` 成
  `projectCharacterLifecycle()`，以 `satisfies Record<AdapterLifecycleState, …>` 窮舉表取代三個 Set，未知原始值退回
  中立「準備中」、不外洩 enum。
- **「現在」頁空狀態**：待決定精確為 0 時明確顯示「目前沒有需要處理的事」（後端只回下限時維持「至少 N 項」）。
- **iPhone 卡片**：新增「重新配對」（導到既有配對區）；未連線時提醒「若桌面的網路位址變了，需要重新配對」；
  進階模式的原始欄位改成「連接診斷」區塊（deviceId、pairedAt、連線狀態與感測旗標原始值），一般模式完全不顯示。
- **agent resume 不得放寬（已知限制 #18）**：`AgentSessionRecord` 新增 `resolvedWorkdir`（canonicalize 後的
  絕對路徑，純加法）；續開時比對工作目錄——不相等、未帶、或舊記錄沒有此欄位的 gateway session 一律
  `PolicyBlocked`；CLI `agents resume` 新增 `--max-cost`／`--max-messages`，省略旗標時自動帶入原 session 的實際
  上限而非 runtime 預設，省略 `--workdir` 時採用記錄的資料夾；桌面「接續上次」優先用 `resolvedWorkdir`。
  兩個把漏洞斷言成預期行為的既有測試改為正確斷言。**Breaking**：升級前建立的 gateway session 無法續開，需重新建立。
- **工作資料夾不得位於 runtime `state/` 底下**：`resolve_gateway_workdir` 的檢查改為雙向；runtime 自己的主動式對話
  工作區搬到 `agent-workspaces/proactive`。
- **iPhone App（模擬器 XCTest 46/46；真機另見 v0.5.1 iPhone 證據文件）**：系統終止後冷啟動會依使用者上次的
  連線意圖自動重連（沿用 1 s→15 s 退避；感測絕不隨之恢復）；連續 4 次連線層失敗或持續 60 秒後顯示固定文案
  「連不上桌面：可能是桌面的網路位址已變更。請在桌面重新產生配對碼並重新配對。」並提供重新配對捷徑；
  TLS 指紋不符與撤銷維持各自文案。

- **Provider 生命週期狀態真正接進執行期（已知限制 #26）**：`ProviderRegistry` 新增 capability→provider 反向索引
  與 `ProviderGate`；`observe_fresh`、push 受器 ingest、`Executor::run_step` 與 `simulate_plan` 都會查擁有 provider
  的狀態——Disabled／Expired／Revoked／Closed 一律拒絕（observe 回 `Unavailable`，execute／simulate 回
  `Blocked{rule:"provider.not-operational"}`）；能力清單投影在 provider 停用時回 `Availability::Disabled`（不新增
  enum 值）。共用能力（多台 iPhone、多個 agent session）只在**所有**擁有者都停下時才擋；Installed／Paired／
  Disconnected 是連線事實不是決定，不擋。升級邊界：舊版落地的 Disabled／Revoked provider 沒有 `provider-off` 標記時，
  第一次重啟即採安全預設（不重開連線、能力停用）、補寫標記並留 `provider.legacy-off-assumed` audit 請使用者確認。
- **首次設定 commit 原子化（已知限制 #19）**：`commit_onboarding` 改為兩段式——Phase 1 純計算（policy／
  preferences／starter recipe YAML 先驗證）；Phase 2a 耐久狀態：policy.yaml 與 starter recipe 檔以 temp+rename 原子寫入並
  保留舊 bytes，SQLite 列（UI 偏好、onboarding.completed、清 draft、audit）在**單一** `Store::transaction` 內提交，
  交易失敗即回寫檔案舊 bytes；Phase 2b 記憶體內能力開關只在 2a 提交後翻，中途失敗把已翻的翻回並在
  `onboarding.partial` audit 說明。故障注入測試涵蓋驗證失敗／policy 寫入失敗／SQLite 交易失敗／元件翻旗標失敗／
  成功只開一個交易。重跑精靈不再把舊草稿疊在期間改過的設定上。殘留：程序在檔案寫入與 SQLite 提交之間崩潰沒有
  journal（見已知限制）。
- **工作交代的送達狀態改由真實證據投影（已知限制 #24）**：新增 `src/work/delivery.ts`，依後端訊號（mailbox 訊息的
  `deliveredAt`、`DomainError` 前綴）投影成六態：已送達／尚未送達（已放進信箱）／排隊中／Agent 不可用／傳送失敗／
  結果不確定，各附一句人話與誠實註記；只有後端蓋了送達戳記才說「已送達」。TaskComposer 與工作卡片的「再交代」共用
  同一份投影。工作狀態標籤對齊產品用語：正在準備／已交給工作助手／處理中／等你回答／等你允許／對方說已完成（保留
  warn 徽章、「對方的說法，尚未檢查」與待你裁決，仍無綠勾）／已由你確認／逾時失敗／失敗／已取消／結果不確定。
- **真正的「只這一次」consent（已知限制 #20）**：`Consent` 新增 `maxUses`／`remainingUses`（serde default；None＝
  沿用 TTL 內不限次），executor 在授權臨界區內原子消耗一次並在同一把鎖內持久化——並行的兩個 plan 只有一個通過、
  dispatch 失敗不歸還、重啟後仍是用掉的狀態、`maxUses=0` 直接拒絕。`POST /v1/session/consent` 加 `maxUses`，
  CLI `session consent --max-uses`，桌面「只這一次」對動器送 `maxUses:1`（並保留 5 分鐘 TTL 當雙重保險，文案改為
  「用過一次即失效；5 分鐘內未使用也失效」）；受器與 tool-operation 的授權仍是短效 TTL——這兩類 scope 帶 `maxUses`
  會被**明確拒絕**（HTTP 400／CLI 錯誤，safety-invariants-058），介面不再回報它不強制的 `maxUses`。
- **`cancel_action` 誠實化**：不再 `let _ = actuator.cancel(..)` 後無條件寫 Cancelled——只有 driver 在 2 秒內確認才是
  `Cancelled`，回錯／逾時／動器不可達一律 `Uncertain`（附 `cancel_unconfirmed` 錯誤），與安全頁「無法取消的會標示
  『結果未知』」一致。內建動器（conversation／通知等）本來就收不回，取消會誠實回 Uncertain。

#### Fixed — 對抗審查 `0c845e0-20260903T185130Z`（62 送審／55 confirmed：high 7／medium 27／low 21；已修 52／部分修 3／未修 0）

完整表格見 `docs/releases/v0.5.1-known-limitations.md` §4；報告 `docs/reviews/adversarial/0c845e0-20260903T185130Z.md`。
每一項都先寫「舊行為下紅燈」的回歸測試再修。摘要（依嚴重度）：

- **high**：session token 只能讀自己 session 的 mailbox（`GET /v1/agent-sessions/{id}/messages` 中介層＋handler 雙層擁有權，
  `MailboxReader::Agent` 在正式環境可達）；intent-only 工具開關收斂成 `gateway::tools_disabled()` 一份真相，codex connector
  對 intent-only session **誠實拒絕**（app-server／exec 都沒有等價 `--tools ""`），主動式對話 `generativeAgent` 只接受
  `claude-code`；**安全 intent 只能 fallback 到安全 intent**（Rust／TS 協商守衛、manifest 驗證拒絕、shu／text／sprite adapter
  以 `envelope.intent` 為準、conformance 逐 intent 斷言；舊 pack 遷移不再產生 `emergency→sleep` 類映射）；角色感測標籤改走
  `statusProjection.sensorKindLabel`（與 tray／首頁／host overlay 同一份投影，iPhone 麥克風不再漏判）；緊急停止逐一 zip 動器結果，
  事件／audit／outbox 帶 `totalActuators`／`unconfirmedActuators`，只有全部確認才說「所有輸出已中止」（計畫罐頭文案改為
  「緊急停止已執行。」）；mobile stop-all 一則都沒送出時不再關去重窗、六個 mobile 動器不再被代簽「已停」；停用高風險受器時
  mobile provider 對仍在串流的手機送 stop，status／tray／overlay 不再無聲。
- **medium**：續開比對**實際生效**的工具開關（零工具→有工具一律 `PolicyBlocked`），桌面／CLI 續開 intent-only session 原樣帶回
  `["conversation.generate"]`，找不到紀錄或缺 `resolvedWorkdir` 一律拒絕；已關閉 agent session 保留 200 筆／30 天並真的刪除
  （`Runtime::prune_agent_sessions`，audit `agent-session.history-pruned`）；TS 視窗 Gateway `renegotiate()` 先把 pending 結清為
  uncertain、安全 intent 補 `system.text`；外部 adapter outbound 的安全訊息等待有配額 8／TTL 5 s，WS 寫入逾時 5 s 即斷線並把
  pending 結清為 uncertain；**宣告即契約**——Runtime 觀察邊界與純 Gateway 進佇列前都擋沒宣告的輸入能力（`InputDropReason::
  CapabilityNotDeclared{requires}`／HTTP 回 `audit-only`＋audit `character.input-capability-not-declared`，TS 端同步）；使魔框不再吞
  點擊；quiet 時永遠只就地眨眼、單元素 ambient 池不再飢餓；所有氣泡回到同一個計時器主人；Roll Call 暫停後一律「停下來了」且
  `onVisibility` 先 suspend 再 beat；Reduced Motion 下光點／逗貓棒不再跟游標；CompanionApp 保留中斷前 ambient 計畫；誠實移除
  假的 utility 競爭（等優先事件為確定性替換）；device cancel 只有裝置回 `not-found` 才是 `NotFound`，其餘 `Uncertain`；
  `memory_list` 回 `total`／`limit`／`limitReached`、來源檢視器一般／進階分層；已配對 iPhone 清單原子寫入＋載入錯誤不再吞、
  配對面板每 2 秒查配對是否被燒掉／到期並顯示原因；幀節奏基準線改為近 5 窗最短間隔中位數（可回升）；`pairing_ever_compared`／
  `pairing_not_recompared`——本連線曾比對過碼的通道不再被標 `pairingUnverified`；lie↔直立過場水平錨點整體位移；`startled-awake`
  接上真實觸發；emergency-stop／stop-all／cancel 的 audit actor 依 token 種類歸因；文件事實勘誤（CHANGELOG 不存在的
  `asyncUtilTimeout` 條目、「v0.5 未發布」、iOS README 25/25、v0.5.0 最終報告缺效能章節）。
- **low**：已關閉且經人工驗證的工作顯示「已由你確認並收尾」；pending 佇列滿時安全 intent 補 `system.text`；移除只寫不讀的
  `restMs`、`carry` 成為真的中繼狀態；感測 banner key 含 `startedBy`、感測標籤對比 ~9.4:1、未知路由顯示「找不到這個頁面」、
  通知中心成為真 modal、狀態列「外觀與語言…」；貼上文字可命名、四處空清單文案真的會顯示；配對面板顯示配對資料全文＋主機位址＋
  複製按鈕（沒有相機也能配）；「輸入→下一幀」改標為 WebView 段（下界＝量測環境幀距，不是 §14 達標證據）、能力矩陣 `<16ms` 改為
  未達標、`reportHitRect` 先做時間閘、隱藏時只保留 CPP sweep 與記帳（狀態輪詢降頻 30 s）；韌體 README 對 pair-locked 的描述改正、
  模擬器浮點序列化鏡射 ArduinoJson；stand↔sit 幾何連續形變（逐幀最大跳動 1.87 px）、組合式通道核心會呼吸；receptor／tool scope 帶
  `maxUses` 直接拒絕。
- **部分修（殘留逐條記在 known-limitations §4.1）**：agent-honesty-024（桌面仍全量重取、無分頁）、protocol-conformance-042
  （證據只存活於 DeviceLink 生命週期）、rig-renderer-046（`not-found` 仍只有預覽格）。

#### Changed — 測試與流程
- **`interaction-api` WebSocket 限流測試決定論化（已知限制 #25）**：改以 `CharacterHub::set_clock` 注入假時鐘，
  精確斷言 50 則接受、第 51 則 rate-limited、每 20 ms 補一則，且 HTTP 回執／事件與 WS 訊息共用同一份預算；
  連續 20 次執行無 flake。限流演算法本身未改。
- **CI**：`desktop-backend` job 從 `cargo check` 升為 `cargo clippy -D warnings`＋`cargo test`。
- **`scripts/release.sh`**：CHANGELOG 沒有 `## [<version>]` 也沒有 `## [Unreleased]` 時直接失敗，不再靜默跳過。
- **`schemas/character-protocol.schema.json`**（golden）：`InputDropReason` 新增 `capability-not-declared{requires}`（純加法；
  由 `GOLDEN_UPDATE=1` 重生）。
- **主動式對話**：`generativeAgent` 只接受 `claude-code` 或 null；既有設定存 `codex` 者，主動式訊息會誠實回「尚未由使用者選擇
  主動式對話 Agent」，桌面選項停用並說明（Codex 無法確定性停用工具）。
- **角色協定契約文件**：README §3.4 步驟 2（安全 intent 只能換安全 intent）、§6（宣告即契約的輸入閘門）、§8（outbound 佇列滿時
  丟剛進來的非安全訊息、安全訊息有界等待）與 adapter-authoring §9／§10 跟上實作。
- **真 Tauri 視窗驗收**：以 debug `.app`＋隔離 home＋fixture agent／fixture iPhone，用 System Events（AX）與
  Core Graphics 真滑鼠事件驗過主視窗、狀態列、精靈、原生資料夾選擇器（取消／選擇／唯讀／寫入二次確認）、
  claimed≠verified、角色視窗顯示／隱藏、空白區點擊穿透與角色本體攔截、緊急停止＋可信覆蓋視窗、感測指示、
  停止所有感測、匯入角色資料夾、外部 daemon 離線覆蓋、完全結束；Reduced Motion（需人類切換 OS 設定）、快捷選單／
  玩具／使魔、adapter 崩潰 fallback 未在真視窗驗收。逐項見 `docs/releases/v0.5.1-test-matrix.md`。

#### 測試（v0.5.1，2026-09-04 於同一台機器實跑；基準為 v0.5.0 tag 重跑）

| 套件 | 基線（v0.5.0） | v0.5.1 最終 | 證據等級 |
|---|---|---|---|
| `cargo fmt --check`／`cargo clippy --workspace --all-targets -- -D warnings` | exit 0／0 warning | **exit 0／0 warning** | — |
| `cargo test --workspace` | 736 passed / 0 failed / 0 ignored（63 target） | **827 passed / 0 failed / 0 ignored（66 target）**；`cargo build --workspace` 成功 | 單元＋真 runtime＋fixture |
| Tauri backend `cargo test`＋clippy | 46 passed / 0 failed | **50 passed / 0 failed**；clippy 乾淨 | 單元 |
| `pnpm typecheck` | 乾淨 | **乾淨** | — |
| `pnpm test`（vitest） | 988 passed / 0 failed（49 檔） | **1168 passed / 0 failed（60 檔）**（+180 測試、+11 檔）。最初的全量跑有 2 個案例只在全量跑失敗、單獨跑通過，根因是 `src/characterName.ts` 的刷新沒有世代概念（測試輔助 reset／prime 之後遲到的舊刷新會把已解析的「小樞」蓋回中立的「角色」；正式執行期行為不變），由 `a6e289e` 修好，修後全量連跑 6 次全綠；對抗審查修復（10 個 commit）後全量再跑 2 次——第一次唯一的失敗是 `regressions-v04` 那條在一般模式斷言原始後端 note 的舊案例（memory-ui-002 的預期紅燈，改為進階模式斷言），第二次 1168/0 | 單元（jsdom） |
| `pnpm build` | 成功 | **成功** | — |
| `./scripts/v03-cli-e2e.sh` | 82 passed / 0 failed | **82 passed / 0 failed** | 真 daemon＋mock 裝置＋fixture |
| `pnpm test:e2e`（Playwright） | 未執行 | **65 passed / 0 failed（2.0 分）** | browser（Chromium；iPhone 相關 spec 對接模擬 iPhone fixture） |
| WS 限流連續 20 次 | — | **pass=20 fail=0** | 單元（假時鐘） |
| `compile.sh`／`--ble` | 兩組態 exit 0 | **兩組態 exit 0**（939 379 bytes／71%；1 190 215 bytes／90%） | fixture（arduino-cli，非真板） |
| iOS typecheck／build／XCTest | XCTest 25/25 | **0 error／BUILD SUCCEEDED／Executed 46 tests, 0 failures**（MotionClassifier 8＋Protocol 17＋ReconnectHint 21） | 模擬器（iPhone 17） |
| iPhone 真機 | 部分驗收（v0.5.0） | **blocked**（鑰匙圈授權待人工；App 未裝上手機，冷啟動測試 0 次執行） | — |
| ESP32 真板 | 未執行 | **未執行**（無實體裝置） | — |
| `pnpm perf` | 60 s +223 KB／10 分鐘 +210 KB | drawRig median **0.100 ms**；stage frame **0.340／0.220 ms**；toy grab median **8.3 ms**（WebView 段）；heap soak after-GC Δ **+510／+446 KB**（60 s）、**+523 KB**（10 分鐘） | browser（headless Chromium，非 WKWebView） |

真 Tauri 視窗驗收（本輪新增的證據等級）與逐列證據見 `docs/releases/v0.5.1-test-matrix.md`。

#### Known limitations（v0.5.1）

完整清單見 `docs/releases/v0.5.1-known-limitations.md`（v0.5.0 的 28 項重新分類為**已修 13／部分修 7／
保留 9**，加上本輪新增的 14 項窄限制與 v0.5.0 文件的 2 處事實勘誤）。摘要：

- **驗證等級**：iPhone 真機驗收**本輪 0 次執行**（`xcodebuild` 停在 macOS 鑰匙圈授權對話框，AI 不得代按）——
  冷啟動自動重連與位址變更提示只有 **iPhone 17 模擬器 XCTest** 證據；ESP32 仍是編譯＋模擬器、BLE 仍是 fixture；
  Reduced Motion、快捷選單／玩具／使魔、adapter 崩潰 fallback **未在真 Tauri 視窗驗收**。
- **範圍邊界**：受器與 tool-operation 的 consent 仍是 TTL（`maxUses` 只計動器派工；這兩類 scope 帶 `maxUses` 會被明確拒絕）；`link-transports-054`
  的修復只涵蓋 unix，非 unix 仍是 bounded-join＋detach＋計數；記憶匯出範圍仍只有記憶項目；受器讀取路徑拿不到
  `pairingUnverified`；`simulate` 已擋但 `transition_provider` 不同步 registry 旗標（gate 在更外層，非安全漏洞）。
- **殘留缺口**：首次設定在「檔案已寫、SQLite 未提交」之間崩潰沒有 journal；角色效能浸泡的 GC 後保留集合
  升到 **+523 KB**（10 分鐘不比 60 秒大，判讀為固定保留集合而非洩漏，但比 v0.5.0 高一倍以上、**來源未定位**，
  列為觀察項）；角色視窗被主視窗遮蔽時 WebKit 暫停繪製造成週期性 re-hello（正常 macOS 行為，非缺陷）；
  故障注入接縫編進正式碼（doc-hidden、未武裝 inert、一次性，**只能讓一次寫入失敗**）。
- **Breaking**：legacy 共享 agent token 不能再 interrupt session；升級前的 gateway session 不可續開；
  `cancel_action` 未經確認一律 `Uncertain`（內建動器的取消一律 `Uncertain`）。遷移見
  `docs/releases/v0.5.1-migration.md`。
- **對抗審查**：55 個 confirmed 已修 52／部分修 3／未修 0，殘留 17 條逐條記在 known-limitations §4.1；修復後**未再跑第二次
  對抗審查**（時間預算），修復由各組紅燈→綠燈測試與最終全套回歸背書。

## [0.5.0] - 2026-09-03 — v0.5 產品重定位（角色・硬體・AI 三核心）

> 已於 2026-09-03 正式發布（tag `v0.5.0`＝`8b713c7`，Release
> https://github.com/miles990/adaptive-interaction/releases/tag/v0.5.0）。發布關卡清單、完整測試矩陣、
> 已知限制、iPhone 真機證據與升級指南見 `docs/releases/v0.5.0-*.md`。

### Phase 9：發布硬化（對抗審查第三輪覆核與修復，2026-09-03）

> 對 v0.5 對抗審查第三輪（`2e02284-20260902T142608Z`，13 維度、136 findings 收斂到 44 筆）的獨立覆核與修復：
> 44 筆 finding 逐一以獨立懷疑者對 commit `521c232` 重新驗證——**43 confirmed／1 already-fixed**
> （`docs-claims-026`：Phase 8 commit `d03e0b9` 已把 acceptance-evidence 的證據跑一節填實，覆核時已不成立）。
> 43 項 confirmed 全數在本分支處理，除下方明列的兩項刻意保留為 partial（範圍已明確劃定、根因已定位）。
> 完整清單、每項的 finding id、回歸測試與「把 bug 放回去」驗證見 `docs/v05-capability-gap-matrix.md` §11 與
> `docs/releases/v0.5.0-known-limitations.md`；測試矩陣見 `docs/releases/v0.5.0-test-matrix.md`。

#### Fixed — link-transports（9 項）
- **MQTT 重連不再重送在飛的命令**：斷線時整個 teardown／重建 rumqttc client＋eventloop，舊 handle 上未 ack 的
  QoS1 publish 隨之作廢，不會在重連後遲到套用到裝置上。
- **ack 逾時／連線中途失敗不再卡在假裝進行中的 Dispatched**：`dispatched_at` 改在真正送出當下蓋章（不是逾時後才蓋，
  修正 executor 算出 age≈0 的問題）；executor 立刻把「送出但結果未知」的收據結算為 `Uncertain`；watchdog 新增
  `Runtime::sweep_receipts_at` 對帳掃描（先判斷 outcome-unknown 再判斷 TTL→Expired）。
- **`LinkError::Reset` 不再誤報 `Failed`**（改 `Uncertain`）——原本會誘發對「結果未知」的重試，可能造成實體效果重複。
- **HTTP 宣告式動器逾時後不再自動重送**：連線失敗（`NotSent`：URL／secret／connect 錯誤，請求確定沒送出）維持既有
  retry；請求送出後才逾時（`OutcomeUnknown`）改為立即回誠實 uncertain，不重送（與 safety-invariants-036 同一根因）。
- **裝置安全上限終於能宣告並到達 Policy `min()`**：宣告式 YAML 新增可選 `limits:`（`maxMagnitude`／`maxDurationMs`／
  `maxPerHour`／`maxPatternSteps`／`maxPayloadBytes`），Serial／MQTT／BLE／HTTP 動器 manifest 一律帶上（與
  safety-invariants-037 同一修復）。
- **MQTT 健康度改成真的追蹤裝置存活**：新增 `RawLink::device_silent_for()`、`LinkReadiness::Stale{silent_ms}`，
  `MqttSpec.livenessTimeoutMs`（預設 15 s＝ESP32 參考韌體 5 s 心跳的 3 倍）；裝置沉默超過視窗才會被判定不健康，
  帶人話原因（「連線還在，但已 N 秒沒聽到裝置」），不再永遠 healthy 到下一次派工。
- **BLE `connected()`／health 現在真的反映斷線**：新增 cancel-safe 的斷線事件監看，裝置離開範圍後不再繼續回報
  healthy 直到下一次派工才發現。
- **estop 更快更不互相拖慢**：`stop_all` 對未連線／連線中的裝置改快速失敗（不再整套 `ensure_open`，serial/mqtt
  不再等 2 s、BLE 不再起 6 s 掃描）；BLE 的 scan guard 改成 cancel-safe，估停被取消也真的 `stop_scan`；
  Runtime 的緊急停止動器階段改成並行執行、有界 ~2 s（原本每台裝置各自最多等 2 s、序列化執行）。
- **id-less 裝置錯誤不再誤歸屬**：平行步驟共用同一裝置時，無 id 的錯誤只有恰好一筆命令在飛才會被當成那筆的結果；
  多筆同時在飛時改列為無法歸屬，逾時結算為 `Uncertain` 而非 `device-refused`（避免把已套用的命令誤標為拒絕而誘發重送）。
- **Serial reader 不再丟棄殘段**：read 逾時（200 ms）時保留已讀到的半行（16 KiB 上限＋警告），不再整段清空造成
  之後的解析失敗。
- **明文憑證現在會被警告**：`pairingCode`／MQTT password 若不是 `secret://` 參照，`build()` 時記 `tracing::warn!`
  並列在 `BuiltCapabilities.warnings`（不阻擋既有 spec，只是不再靜默）。

#### Fixed — safety-invariants（5 項）
- **「停止所有感測」／緊急停止現在真的到達 iPhone**：對每台已連線手機送 `stop-all{sensors:true}` 並等待（≤2 s）
  `ack{stopAll:true}` 或後續 `status{micLevel:false}` 確認，本機麥克風＋每台手機逐一誠實回報
  stopped／unknown／unreachable，UI 與 audit 都拿得到整份 `StopAllSensorsReport`，不再只憑本機狀態就宣稱
  「已停止所有感測」。
- **iPhone 麥克風事件改成真的變化才發**：`sensor.started`／`sensor.stopped` 只在 `micLevel` 真正改變時發出（不再
  被 30 秒心跳洗版），斷線時補一則 `sensor.stopped{reason:"disconnected"}`；SSE 訂閱者不再永遠看不到。
- **HTTP 宣告式動器逾時不再自動重送**（同 link-transports）。
- **裝置安全上限終於能到達 Policy `min()`**（同 link-transports）。
- **agent token 不再能經 `/v1/providers` 讀到 iPhone 的公開身分指紋**：`mobile_register_provider` 改用
  `sha256("mobile-identity:v1:{deviceId}:{tokenHash}")` 衍生指紋（不是驗證用的 token_hash 本身）；
  `providers_list`／`provider_get` 對非人類 principal 一律抹掉 `identity.fingerprint`，補上原本「`/v1/mobile`
  讀不到就安全」的邊界漏洞。

#### Fixed — mobile-server（8 項；1 partial 見下）
- **emergency／verified-success 兩個真相狀態收回 Runtime 專屬**：新增 `RUNTIME_ONLY_STATES`，AI 再也不能經
  `character.present` plan 讓 iPhone 顯示「緊急停止中」；Runtime 自己的 estop／解除改為並行投影到每台已連線
  手機（≤1.5 s 有界等待，逐台 acknowledged／refused／unknown／unreachable，寫入 `mobile.character-emergency`
  audit）。
- **DoS 面補齊並在 `mobile_status` 誠實顯示**：連線數上限 8（`MOBILE_MAX_CONNS`，超過在 accept 當下拒絕）、
  TLS／WebSocket 交握 5 s 逾時、未認證連線 10 s 死線（`mobile.unauthenticated-timeout` audit）、單則訊息
  128 KiB 上限（原本是 tungstenite 預設 64 MiB）、每連線 30 msg/s 速率限制（超過關閉並記
  `mobile.rate-limited`）；heartbeat 新增 `authTimeoutMs`／`maxConnections`／`refusedConnections`／
  `maxMessageBytes`／`maxInboundPerSec`。
- **accept loop 對暫時性錯誤改退避重試**：新增 `accept_error_action()` 分流暫時性／致命錯誤（`EMFILE`／`ENFILE`
  等在 Rust 沒有穩定 ErrorKind，一律當暫時性重試；只有 PermissionDenied／AddrInUse 等或連續 20 次才停）；真的
  停下來時誠實回報 `started:false`、`port:null`、關掉 Bonjour 廣播並記 `mobile.server-stopped`（不再讓 status
  假裝仍在運作）。
- **多台 iPhone 動作必須指定目標**：`pick_conn` 不再挑 BTreeMap 第一台——只有恰好一台連線才能省略 `deviceId`，
  兩台以上一律誠實拒絕並列出目前連線中的 id；收據的 `driver_response` 帶上實際送達的 `deviceId`／`deviceName`。
- **TLS 私鑰改成原子式 0600 落地＋載入時檢查**：新增 `write_private_key`（`create_new`＋`mode(0o600)`，先寫暫存檔
  再 rename，不再有先 0644 再 chmod 的空窗）與 `ensure_key_is_owner_only`（載入既有金鑰時權限過鬆會嘗試修復，
  修不動就誠實拒絕啟動而不是帶著鬆散權限的私鑰繼續跑）。
- **`pending_acts` 不再永久殘留**：等待端 future 被丟棄（HTTP client 斷線／CLI Ctrl-C）或手機斷線時，
  新的 `PendingGuard`（Drop 時移除表項）與 `fail_pending_for_device` 保證清空，在途動作立刻以誠實的
  `dispatched＋outcomeUnknown＋failed(iphone-disconnected)` 收場。
- **伺服器端參數驗證對齊 iOS App**：新增 `validate_wire_params`，haptic／notify／tts／torch／flash／
  character.present 的規則（字數上限、數值範圍、列舉白名單）與 App 端一致，不再只能靠手機拒絕才浮現規則落差。
- **`mobile_present_verified` 誠實化**：手機回非 `ack`（含與 stop-all 競態的 `stopped`、App 的 bad-state）時
  誠實回 `Err`（不再吞成 `Ok`）；同一類問題也修在 `mobile_ble_scan`（手機回 err 不再被當成掃描結果）。
- **Partial（`F-043` 多台 iPhone）——已於第二輪修復**：目標選擇與收據歸屬已修好；`executor.rs::
  note_capability_tested` 原本仍透過「字典序第一個 provider」記「已測試」證據，第二輪對抗審查覆核時發現
  這一半實際上也已在 HEAD 修好（`executor.rs` 取 driver 收據的 `deviceId` 呼叫
  `note_capability_tested_on`），見下方「對抗審查第二輪」與 `docs/releases/v0.5.0-known-limitations.md` §5。

#### Fixed — character-protocol（4 項）
- **Reduced Motion 改成每個 instance 各自真實協商**：新增 `Gateway::set_reduced_motion`／`reduced_motion`，
  `hello_for` 用它協商（不再是永遠 `false` 的寫死值）；`CharacterHelloInput`／`CharacterHelloBody`／IPC／
  `CharacterInstanceView` 新增 `reducedMotion`；回執 resolution 只能降級不能升級（adapter 可誠實降級為
  `reduced`，不能自己升回 `exact`），修正回執與 `GET /v1/character/instances` 把 `reduced` 誤報成 `exact`。
- **TS Gateway 的 merge 分支不再說謊**：合併進既有演出的 intent 改誠實回 `cancelled{reason:"merged"}`（不再假裝
  從未演出過的 `completed{merged}`）；duplicate／alreadyTerminal 回執改帶原命令協商出的真實 resolution（不再
  硬編 `exact`），與 Rust 端的 `cancelled{merged}` 行為一致。
- **50 則/s 限流改成每個 instance 一份真正共用的預算**：新增 `Gateway::allow_message()`／`note_wire_rejected()`，
  HTTP 回執／事件與 WS 畸形訊息全部算在同一份預算內（外部 adapter 之前可以繞過 WS 限流無上限灌 audit 與
  `companion.click` 觀察）；畸形訊息稽核有界（同一 instance 每 5 秒最多一列，`suppressed` 記被壓下的次數）。
- **純聲音／燈光角色能真的表達工作／等待／未知**：`intent_capabilities` 為 Think/Wait、Work、Unknown/Cancelled
  補上 `audio.speech`／`audio.effect`／`haptic.cue`——之前只宣告 `audio.*`／`haptic.*` 的角色，work/think 會被
  誤判 `unsupported`、wait/unknown 會被誤判走 `system.text`，與文件承諾相反。
- 附帶新增：第三方 manifest conformance 測試套件（`conformance.rs`，涵蓋 bundled＋`examples/character-adapters/`
  ＋`CPP_CONFORMANCE_MANIFESTS` 指定的外部 manifest）；斷線（crash／transport-closed／goodbye／revoke）時進行中的
  安全 intent 改為交接給 `system.text`（原本 README §9 承諾的 fallback 並不存在，進行中的安全 intent 會直接消失）。

#### Fixed — ia-settings（6 項；1 partial 見下）
- **首次設定精靈「進一步自訂」不再寫入沒有主人的設定**：移除一般模式的 `channelLimits["*"]`／
  `requireApprovalAt` 孤兒控制項（「主動打擾次數」標籤與實際效果本來就不符），只保留安靜時段。
- **重新執行精靈不再靜默停用已啟用的非新手能力**：例如已配對 iPhone 的動作／電量／觸碰感測，重跑時會保留現況
  而不是退回 beginner-safe 預選；新增 `POST /v1/onboarding/preview` 零副作用試算端點，精靈「完成設定」前先彈出
  「套用前確認」對話框列出真的會變動的項目（差異來自後端試算；試算失敗才退回本機估算並在畫面上明說）。
- **收件匣 `pendingCount` 不再只看最近視窗**：新增 `Store::receipts_with_status`，直接依狀態（`uncertain`／
  `blocked`）查詢開放收據（上限 1000），不再只從最近 200 筆歷史裡數；新增 `pendingCountExact` 誠實旗標，撈滿
  上限時回 `false`（代表 pendingCount 是下限，不是總數）。
- **窄視窗「更多」選單現在會正確高亮目前所在的子項**（原本折疊後的 anchor 與選單比對的 id 對不上，五個細項與
  「更多」自己都永遠不會亮）。
- **解除緊急停止對話框改三段式**：會恢復可用／不會自動恢復／你先前已停用因此仍為停用——不再把使用者自己
  停用的動器列為「解除後會恢復可用」。
- **風險分級修正本機音效／語音朗讀**：從 L2「會用到你的檔案、偏好或記憶」降為正確的 L1（`companion.sound.play`／
  `companion.speak` 沒有外部或實體效果，不該套用需要檔案存取的風險文案）。
- **Partial（`ia-settings-012` 安靜時段預設封鎖桌面角色）**：角色頁的安靜時段編輯器已送出明確的靜音通道清單
  `["audio","haptic","notification","light"]`（不含 `desktop-pet`），L0 呈現不再被安靜時段誤判為「待你決定」；
  Rust 根因（`activity.rs` 的收件匣投影，`receipt_item()` 已改為排除純呈現動器的 blocked 收據，只有卡在
  「需要人類核可」的呈現動作才計入待決定）已修。**首次設定精靈那一側未修**——`Onboarding.tsx` 仍寫
  `silencedChannels: []`，從精靈建立的安靜時段預設仍會靜音桌面角色（DEFAULT_QUIET_SILENCED 含
  `desktop-pet`）。

#### Fixed — agent-honesty（1 項）
- **緊急停止不再偷偷開一輪新的模型呼叫**：`estop_agent_sessions` 不再把「cancel」包裝成 `ToSession` 訊息送進
  每個 gateway agent（原本會觸發 codex `turn/start`、claude 新的 user 訊息、甚至整個新的 `codex exec` 程序，
  誤發 `fetched` taxonomy、消耗一則 message 預算，且每個卡住的 session 最多拖 5 秒才被強制終止）。改為直接
  關閉 session（撤銷能力＋清空未送達信箱＋終止程序樹）並在事後補一則不送達 agent 的 `from-session` 稽核訊息
  （`kind:"emergency-stop"`，`deliveredToAgent:false`，不占 message 預算）；所有 session 改為並行終止
  （`futures_util::future::join_all`，每個 ≤2 s 有界），實測兩個卡住的 session 從 10.0 s 降到 5.8 ms。

#### Added — 前端 IA 結構性改動（任務要求，非對抗審查 finding）
- **工作頁「開始前預覽」從固定六項收斂為三個回答**：這次會讀取什麼／會不會修改內容／最多使用多少時間與費用，
  其餘（使用哪個 Agent、工具、沙箱、時間／訊息／費用上限、如何取消、原始授權範圍）移入可收合的
  「查看技術細節」；讀取範圍文案改為誠實描述（系統擋得住「改不到別的地方」，不宣稱「只讀取這個資料夾」的
  硬邊界）；換資料夾會使先前的寫入第二次確認作廢；續租寫入權限的工作改為二次確認（「確認延長（含修改權限）」）。
- **原生資料夾選擇器**：`apps/interaction-desktop/src-tauri` 新增 `tauri-plugin-dialog`（僅 `main` 視窗的
  capability，未註冊 `tauri-plugin-fs`、未給任何 `fs:*` 權限，WebView 只拿到一段路徑字串）；`pickDirectory`
  改回傳 `picked／cancelled／unsupported／error{message}` 四態，瀏覽器版改成誠實的靜態說明（「瀏覽器版沒有
  原生資料夾選擇器；請貼上資料夾路徑。」），不再是點了沒反應的假按鈕。
- **連接與權限第一層改為裝置優先五區**：已連接的裝置／系統可以看見什麼／系統可以做什麼／目前需要確認的
  權限／立即停止與撤銷（固定順序）；iPhone 裝置卡片新增「停止感測」「測試連接」按鈕，用字遵守誠實階梯
  （「已要求停止（以手機回報為準）」≠「已停止」；「有回應」≠「已測試」）；新增每機端點
  `POST /v1/mobile/devices/{id}/sensors/stop`、`POST /v1/mobile/devices/{id}/test`。
- **角色頁一般／進階分層**：一般模式只顯示內建／第三方、可接收、已測試三個徽章，額外授權（可執行程式／
  需要網路）只給一句人話說明；外部／可執行程式／需要網路徽章與執行位置欄位移到進階模式；一般模式匯入角色
  只有選檔（移除貼上原文的輸入框），驗證器原文收在收合的「問題明細」裡；不認得的互動能力 id 不再外洩原始
  字串（顯示「其他互動」）。

#### Added / Changed — 對外契約（完整清單見 `docs/releases/v0.5.0-migration.md`）
- 新端點：`POST /v1/onboarding/preview`（零副作用試算，回應形狀同 commit 的 diff）、
  `POST /v1/mobile/devices/{id}/sensors/stop`、`POST /v1/mobile/devices/{id}/test`。
- `POST /v1/sensors/stop` 回應形狀改為 `{stopped, uncertain, local:{microphone}, devices:[{deviceId,name,
  outcome,waitedMs,via}]}`（原本是 `{stopped:true}` 固定值）。
- `POST /v1/emergency-stop` 的 payload／事件／audit 新增 `sensors`（`StopAllSensorsReport`）與
  `characterEmergency`（`[{deviceId, outcome}]`）。
- `activity_inbox` 回應新增 `pendingCountExact: bool`。
- `GET /v1/providers`／`/v1/providers/{id}` 對非人類 principal（agent／session／character adapter token）
  省略 `identity.fingerprint`。
- 宣告式 YAML 新增可選欄位 `limits:`（`CapabilitySpec`，maxMagnitude／maxDurationMs／maxPerHour／
  maxPatternSteps／maxPayloadBytes）與 `mqtt.livenessTimeoutMs`（預設 15000）。
- 新環境變數 `INTERACT_AI_MOBILE_ADVERTISE`（`0`／`false`／`off`／`no` 關閉 Bonjour 廣播並只綁
  `127.0.0.1`；供 E2E／CI 使用，避免區網廣播）。
- iOS：`ServerMessage.stopAll` 新增 `reason`（`user`／`emergency`，缺省或未知一律 fail-safe 為 `emergency`）；
  `ClientMessage.ackStopAll` 新增回聲 `sensors` 欄位。桌面端於第二輪修復（見下）補上送出 `reason`；桌面端
  尚未消費 ack 回聲的 `sensors` 欄位（已知限制，見下）。
- `CharacterHelloInput`／`CharacterHelloBody` 新增 `reducedMotion`。

#### Known limitations（第一輪，已由第二輪修復部分項目——見下方「已於第二輪修復」）
- `ia-settings-012` 精靈那一側未修（見上，第二輪覆核仍為真，保留）。
- MQTT 重連不重送只在內嵌 `rumqttd` broker 上驗證，沒有真實 ESP32 board 上的重送測試。
- BLE 斷線偵測只有假事件流（`futures::stream::iter`）的單元測試，零真實周邊驗證；Linux 仍誠實拒絕 BLE。
- HTTP 逾時分類保守地把「TLS 握手失敗」等 reqwest 未分類錯誤也歸類成 `OutcomeUnknown`（方向安全：不會把真的
  沒送出的請求說成已送出；代價是可能把少數真的沒送出的請求也標成不確定而非可重試的失敗）。
- 原生資料夾選擇器（`tauri-plugin-dialog`）沒有任何自動化驗收：vitest 的 `invoke` 是 mock，Playwright 跑
  瀏覽器版（無 Tauri IPC），只通過 `cargo check`／`clippy`／`cargo test` 的編譯驗證，需要桌面手動驗收。
- iOS 新增 Xcode 專案（`InteractionCompanion.xcodeproj`）＋裝置腳本（`device-build.sh`／`device-acceptance.sh`）；
  詳見下方「對抗審查第二輪」——真機部分驗收已完成（2026-09-03，iPhone 11／iOS 26.3.1），XCTest 修正為 25/25。
- 本輪未 push、release、deploy、開 PR 或建立 commit（依 repo 規則需使用者明確授權）。

**已於第二輪修復、不再是限制**（第一輪誤留或第二輪修復；避免文件漂移重犯，完整說明見
`docs/releases/v0.5.0-known-limitations.md` §5）：`F-043` executor 已測試證據歸屬；
`credential_warnings()` 未轉發進 provider 紀錄；`mobile_ble_scan` 沒有 `deviceId` 參數；桌面端未送出
stop-all 的 `reason`；iOS `ActuatorCenter.stopAll` 不會設 emergency 角色狀態。

#### 對抗審查第二輪 `c3d1786-20260903T124638Z`：78 reviewed／74 confirmed／4 refuted；63 fixed、4 partially-fixed、7 docs-claims fixed in docs（2026-09-03）

- **最終回歸抓到的額外缺陷（已修）**：`StateView` 在 `useAsync` 背景重新整理時把清單換成「載入中…」，底下的工作卡整個卸載重掛，使用者展開的訊息面板與核可「拒絕」的裁決結果（「你已拒絕」）在每一次 SSE 事件觸發的刷新時消失（Playwright `work-delegate` 拒絕列復現）。現在只有第一次載入顯示載入中，之後保留舊資料（`aria-busy`），更新失敗顯示「更新失敗（顯示的是上一次的資料）」；`useAsync` 失敗時保留 `data`。回歸測試 `src/test/stateView.test.tsx`。
- **CLI E2E 腳本**：agent 工作資料夾改用獨立暫存目錄（Runtime 自本版起拒絕把自己的狀態資料夾當 workdir，見 agent-honesty-022）。

> find＝opus、verify＝sonnet；74 項 confirmed 中 63 項 fixed、4 項 partially-fixed（根因已定位、範圍已
> 明確劃定，見 `docs/releases/v0.5.0-known-limitations.md` §1）、7 項 docs-claims（文件與程式碼不符）直接
> 訂正在文件本身（本節與各 `docs/releases/v0.5.0-*.md`）。workflow 的 persist 步驟因 API 529 失敗，本節與
> 對應報告由 integrator 依 workflow 執行結果人工落盤。完整 finding 清單見
> `docs/reviews/adversarial/c3d1786-20260903T124638Z.{md,json}`；回歸測試與「把 bug 放回去」驗證見各
> finding id 對應段落。

##### Fixed — memory-ui（6 項，全 fixed）
- **`memory-ui-001`**：控制中心「忘記這些」不再被角色視窗自己的舊記憶副本復活——`companionInteractionMemory`
  改為 Runtime／host prefs 是唯一真相來源，每次 companion-reload 都從 prefs 重建。
- **`memory-ui-002`**：`memory_context_bundle` 不再靜默丟資料——新增 `excluded.overCapacity`／`truncated`／
  `limits`／`note`，被卡住的項目與掃描上限現在都會誠實回報，UI 顯示「這份不是完整的」。
- **`memory-ui-003`**：`memory_export` 誠實聲明範圍——`scope:"memory-items-only"`／`notIncluded`／
  `limitReached`；UI 按鈕改名「匯出記憶」並說明知識／素材／角色互動記憶不在匯出範圍內。
- **`memory-ui-004`**：Agent 建立的 user-memory／persona-core 重新確認按鈕改顯示真實可延展天數（30 天，不是
  會被 Governor 砍掉且降級的 90 天），送出後對照後端實際回應誠實顯示是否被縮短或降級。
- **`memory-ui-005`**：記憶頁每個會呼叫後端的按鈕（清除短期記憶、知識「不採用」、素材操作）補上
  try/catch，失敗會顯示 `role="alert"` 訊息，不再是無聲的 floating promise。
- **`memory-ui-006`**：一般模式不再外洩 10 層技術分類——新增人話分組（你告訴我的事／角色的設定／學到的
  知識／工作與任務／這次對話的暫存＋其他），進階模式維持原始技術分層與標籤。

##### Fixed — mobile-server ＋ safety-invariants（provider 生命週期）（7 項，全 fixed）
- **`mobile-server-061`**：緊急停止中連上／重連的 iPhone 現在會收到 `stop-all{sensors:true,reason:emergency}`
  並被有界等待確認（不再只送純 UI 的 `character.present emergency`）。
- **`safety-invariants-074`**：宣告式裝置 provider 的停用／撤銷現在跨重啟持久——新的 `provider-off:<id>`
  store meta 標記讓 `register_declarative_spec` 在 `build()` 前就關連線、強制停用受器／動器（升級邊界殘留，
  見已知限制）。
- **`mobile-server-062`**：`mobile_ble_scan` 補上與其他手機動作一致的 `is_estopped()` 閘門，緊急停止期間不再
  送出 BLE 掃描指令。
- **`mobile-server-063`**：一台手機斷線／撤銷時，其他仍在串流的手機不再從 `status.activeSensors` 無聲消失——
  新增 `mobile_stop_other_streaming_phones()` 逐台誠實要求停止並等待確認。
- **`mobile-server-064`**：撤銷一台正在串流的手機現在會立即結束其感測與在途動作（`mark_disconnected`＋
  `fail_pending_for_device`），不再要等到 4 秒 ACT_TIMEOUT。
- **`mobile-server-065`**：觀察訊息頂層的 `at` 時間戳現在會併入 facts，manifest 宣告的 `provides:["event",
  "at"]` 不再是空話。
- **`mobile-server-066`**：認證失敗（未知裝置／錯 token／已撤銷手機重連）現在會寫 `mobile.auth-failed`
  audit（含 peer／deviceId／knownDevice，絕不含 token）並計入 `status.heartbeat.failedAuths`。

##### Fixed — agent-honesty ＋ SSE 邊界（6 項，5 fixed／1 partial）
- **`agent-honesty-022`**：「接續上次」不再悄悄放寬時間／費用／資料範圍上限——`resolve_gateway_workdir`
  拒絕任何指到 runtime state 目錄的 workdir，`create_agent_session` 對已知的原始 session 強制續租不得比原本
  寬（省略欄位＝變寬也算）。
- **`agent-honesty-023`**：`codex_exec.rs` 的結束事件判讀抽成純函式 `drain_outcome_event`——只有觀察到非零
  結束碼才算 `TaskFailed`，其餘（no terminal event、訊號終止、無法讀取結束碼）一律 `TaskOutcomeUnknown`。
- **`agent-honesty-024`**：委派動作的收據現在真的能到 `Acknowledged`——透過 `DriverReceipt` 的
  `.dispatched().acknowledged()` 路徑（而非放寬 executor 的 Dispatched 守衛，那樣會被 executor 自己的合併
  邏輯洗回去）。
- **`agent-honesty-025`**：`mailbox_send` 誠實回報是否真的送達 agent（`deliveredAt` 有無），AiPage 據此區分
  「已送達 Agent，尚未完成」與「已放進信箱，尚未送達」兩種文案。
- **`agent-honesty-026`**：非 gateway agent 真的取走任務時會發出 `fetched` taxonomy 事件（原本只有 gateway
  session 會發，人類只是看一眼信箱不會誤觸發）。
- **`safety-invariants-078`（partial）**：`EventType::AgentSessionState` 加進 SSE `event_allowed` 排除清單，
  legacy agent token 不再能經 SSE 側通道讀到 agent session 狀態；`POST /v1/agent-sessions/{id}/interrupt`
  仍對任何 session id 放行未修（見已知限制）。

##### Fixed — ia-settings ＋ 前端 IA（10 項，全 fixed）
- **`ia-settings-012`（此編號為第二輪新 finding，非同名第一輪 partial）**：安全頁「解除緊急停止」現在導覽到
  時會自動捲動並取得焦點，重複導到同一路由也會換一把新的掛載 key 讓子頁狀態重置。
- **`ia-settings-013`**：統一收件匣不再直接印 `裝置 {deviceId}`——改用 `inboxDeviceLabel` 轉成「你的
  iPhone」或能力名稱，原始 id 只留在進階模式。
- **`ia-settings-015`**：收件匣標題在後端只回下限時誠實顯示「待決定 至少 N 項／共 N」，不再說成總數。
- **`ia-settings-016`**：收件匣的感測開始／停止標題不再外洩原始裝置 id（`sensor_event_label` 一律轉成人話
  或安全的 fallback）。
- **`ia-settings-017`**：ActivityPage／GlobalSearch 的收據意圖標籤共用 `receiptIntentLabel`，一般模式不再顯示
  原始 runtime intent 字串。
- **`ia-settings-018`**：首次設定精靈套用失敗時不再合併成一行——誠實列出已套用／失敗於哪一步／未嘗試的
  項目，且不假裝自動還原（`onboarding.partial` audit；仍非原子，見已知限制）。
- **`ia-settings-019`**：安全頁「重新驗證」按鈕的結果與失敗都會顯示訊息，不再是無聲的 floating promise。
- **`ia-settings-020`**：首頁開始／結束工作階段失敗時會顯示 `role="alert"` 訊息。
- **`ia-settings-021`**：GlobalSearch 的知識收據標籤與網域包 id 只在進階模式顯示原始值。
- **`safety-invariants-075`**：L4 高風險能力的同意對話框依風險分級決定選項——tier ≥ 4 不再有「整個工作階段」
  選項，預設改為 5 分鐘短效授權。

##### Fixed — 角色 rig／perf／Director（17 項，全 fixed）
- **`rig-renderer-056`**：跨姿勢交叉淡出新增 `poseFrom` 通道，混合目標本身也在轉場中時的最差單幀頭部跳動從
  14.14 px 降到 4.48 px（未到 0，見已知限制）。
- **`rig-renderer-058`**：`stand↔sit`／`stand↔crouch` 等非 lie 姿勢過渡不再在插值中點硬切，最差單幀跳動從
  10.00 px 降到 0.64 px。
- **`director-pipeline-044`**：新增 `CLEAR_PROTECTED_TRANSIENTS`，快速連點／取消／舊版 idle 動畫路徑不再能
  清掉正在等待使用者確認的 `requesting-consent` 狀態；緊急停止仍會清掉它。
- **`companion-gameplay-032`（partial）**：舞台上角色與玩具之間的死區改成正常視窗互動而非吞掉點擊；Tauri
  單一 hit-rect 仍未拆成多矩形（見已知限制）。
- **`companion-gameplay-033`**：Reduced Motion 下的招呼／走動使魔會真正收斂到靜止並清除愛心圖示，不再永久
  卡在「打招呼中」。
- **`companion-gameplay-034`**：`spawnToy` 改用 `world.nextToyId` 是否真的推進判斷成功，玩具滿額時丟出新玩具
  不再假裝成功並寫入互動記憶。
- **`companion-gameplay-035`**：新增真正的 `greet-familiar` 玩法——角色會主動走向使魔打招呼並收到回應，不再
  只是轉頭看一眼。
- **`perf-claims-007`**：`StageRenderer.loop()` 補上 Reduced Motion 靜態短路，量測到 361 tick 只畫 7 幀。
- **`perf-claims-008`**：新增以顯示器自身 rAF 節奏基準線比較的 pacing 降級判斷，raster／compositor 卡頓現在
  也能觸發 30 fps 降級（不只是 JS 成本超支）。
- **`perf-claims-009`**：`pnpm perf` 的 soak 量測範圍擴大到涵蓋真 `CharacterGateway`（真 shu adapter）、
  `InteractionDirector`、behavior／記憶與 500 筆事件環，並在輸出誠實列出涵蓋與不涵蓋的範圍。
- **`companion-gameplay-036`／`rig-renderer-060`**：移除死程式碼 `behavior.scheduleMicroAction`／
  `SHU_MICRO_ACTIONS`（無production caller，內含會誤點亮工作狀態通道的隱藏假資料）。
- **`companion-gameplay-037`**：連續戳弄的反應從單一字串擴充成 3 個變體池＋更短的冷卻，不再每 8 秒重播同一
  段演出。
- **`rig-renderer-059`**：Reduced Motion 下 `ExpressionTimeline` 的自動眨眼完全關閉（原本每 2.2–5.4 秒仍有
  0.05 的眼睛開合抖動），`blinkNow()` 仍接受提示但不再實際渲染。
- **`director-pipeline-045`**：Host 端判斷是否為眨眼演出改用新的 `DirectorAction.source === "blink"`，不再
  硬編比對特定角色的表情 id 字面值。
- **`director-pipeline-046`**：移除 `InteractionDirector.score()`（零呼叫者的死程式碼），並在 `director.ts`
  header 誠實記錄 Utility Scoring 實際只用在 `machine.ts` 的同優先權 tie-break，不是決策主路徑。
- **`perf-claims-011`**：`reportHitRect()` 在舞台暫停／銷毀時提早返回，不再每 500 ms 心跳仍觸發 Tauri IPC。

##### Fixed — Character Presentation Protocol（8 項，全 fixed）
- **`character-protocol-038`（blocker）**：安全 intent 的每一種非 completed 終態（不只 `failed`）現在都會
  落到 `system.text`，包含斷線與重新協商時尚未結算的安全 intent（sweep 也會補送）。
- **`character-protocol-039`**：adapter 在安全 intent 進行中重新 `negotiate` 不再讓在飛的安全提示消失——
  `on_negotiate` 現在會把它們透過 `resend_safety_as_system_text` 重送。
- **`character-protocol-041`**：TS 的 `INTENT_CAPABILITIES` 改回逐 intent 對照表並用 Rust 產生的 golden JSON
  雙邊斷言，不再是可能與 Rust 權威表不同步的單一共用清單。
- **`character-protocol-040`**：adapter 回報的 `Unsupported`／`Failed` resolution 不再被 gateway 丟棄改寫成
  `Reduced`，收據不會再說「什麼都沒演出」卻標成 `exact`。
- **`character-protocol-042`**：AI 的 `character.present` 這類命令現在只由桌面 instance 結算，外部 adapter
  不能再用自己的回覆搶先決定收據結果。
- **`character-protocol-043`／`safety-invariants-077`**：外部 adapter 的輸入事件不再能合成
  `companion.quick-action`／`companion.click` 等人類互動觀察（只留 `character.input-not-observed` 稽核），
  堵死了繞過桌面 surface 閘門的路徑。
- **`safety-invariants-076`**：adapter 在緊急停止期間（重新）連線／協商時會立刻收到緊急投影＋
  `character.estop-resync` 稽核，不用等下一次事件才追上真相。

##### Fixed — link-transports ＋ protocol-conformance（13 項，11 fixed／2 partial）
- **`link-transports-047`**：宣告式 HTTP 動器的 `emergency_stop` 不再無條件回 `Ok(())`——沒有 stop 端點、
  或裝置拒絕／逾時都會誠實回錯，緊急停止的 `stoppedActuators` 不會再算進從未被要求停止的裝置。
- **`link-transports-048`**：`ensure_ready` 補上世代檢查，重連期間收到的舊握手不會被誤蓋到新連線上。
- **`protocol-conformance-029`／`link-transports-053`**：`InFlightGuard` 現在涵蓋 `cancel()`／`read_state()`／
  `stop_all()`，並行請求時無 id 的裝置錯誤不再被誤歸屬給恰好在飛的那一筆命令。
- **`protocol-conformance-027`**：ESP32 序列模擬器不再因超出 float32 範圍的參數而整個程序當掉（改成
  clamp＋誠實錯誤，與韌體 `roundToLong`＋`clampLong` 行為一致）。
- **`protocol-conformance-028`**：韌體與桌面兩端補上 BLE notification 的長度紀律（`setMTU`＋分段＋換行
  終止符；桌面端新增 `NotifyAssembler` 重組並計數解不出來的訊息）。
- **`link-transports-049`**：serial 埠立刻掛掉的連線現在會退避並在連續兩次「活不到 1 秒」後回報離線，不再
  在 Connecting 狀態原地打轉。
- **`link-transports-050`**：一次讀不到任何 manifest 宣告的 fact 不再算是成功觀察——改回 `Unavailable` 並
  記健康度失敗，runtime 不會再把零 fact 的回覆標成「已測試」。
- **`link-transports-051`**：裝置的配對鎖定（`pairingLocked`）現在會被辨識為「鎖定中，不是密碼錯」，不再
  誤報成配對碼錯誤。
- **`protocol-conformance-030`（partial）**：host 端現在會誠實記錄「配對碼從未被比較」
  （`pairing_unverified`），但 `providers.rs` 尚未依此降級 evidence level（見已知限制）。
- **`link-transports-052`**：逾時訊息現在會附帶「N 則訊息在等待期間被丟棄／解不出來」的說明，不再是單純
  的「沒有回應」。
- **`link-transports-054`（partial）**：serial pty/file fallback 的讀取執行緒洩漏現在會被計數＋警告，尚未
  消除根因（見已知限制）。
- **`protocol-conformance-031`**：ESP32 韌體 README 的參數範圍表訂正為與硬限制表一致（`servo.move` 是
  10..170，不是 0..180）。

##### Added — 測試（`docs-claims-071`：先前未記錄的既有新增）
- **Playwright user-task 套件**：12 個 spec、65 個 `test(`（8 個新增：`a11y`／`agent-not-installed`／
  `character`／`estop`／`home-state`／`iphone`／`sensors`／`work-delegate`，加上既有
  `app`／`evidence`／`narrow`／`offline`）；`playwright.config.ts` 新增三個有序 project
  （`first-run`→`main`→`estop-last`，破壞性列放最後）；共用 `e2e/helpers.ts`。
- **`examples/fake_iphone.rs`**：程序外**【模擬 iPhone（fixture）】**可執行檔，供 Playwright 的
  `iphone.spec.ts` 等 4 個測試重現手機連線／斷線／權限拒絕／停止感測未回應等狀態；不是真機證據。

##### Known limitations（第二輪新增；完整清單與根因見 `docs/releases/v0.5.0-known-limitations.md`）
- `safety-invariants-074`：`provider-off:<id>` 標記在升級邊界有一次性缺口——舊版本停用的裝置在升級後第一次
  重啟仍會重開連線，之後每次明確停用／啟用才會正確持久。
- `link-transports-054`：serial pty/file fallback 的讀取執行緒洩漏已計數（`detached_reader_threads()`），
  根因（blocking read 無逾時）未消除，只影響 fallback 路徑，不影響真硬體的 `serialport` 路徑。
- `protocol-conformance-030`：host 已標示配對碼未被比較（`pairingUnverified`），`providers.rs` 尚未依此降級
  handshake evidence level。
- `companion-gameplay-032`：舞台死區已消除，但 Tauri `companion_hit_rect` 仍只回報單一矩形，該區域仍非桌面
  可點穿。
- `rig-renderer-056`：跨姿勢交叉淡出的最差單幀頭部跳動降到 4.48 px，未到 0（pose 通道仍是兩姿勢混合的
  近似）。
- `agent-honesty-022`：resume 的原始 workdir 未持久化到 `AgentSessionRecord`；CLI `agent open --resume` 省略
  `--ttl`／`--max-cost` 時會被誠實拒絕，但尚未自動帶入原始限額。
- `safety-invariants-078`：`POST /v1/agent-sessions/{id}/interrupt` 對 legacy agent token 仍未做擁有權比對。
- `memory-ui-003`：`memory_export` 仍只涵蓋記憶項目（不含知識／素材／角色互動記憶）；`limitReached` 是「這頁
  剛好裝滿」推得，非精確計數。
- `ia-settings-018`：首次設定精靈的 `commit_onboarding` 中途失敗時已誠實回報進度，但仍非原子（已套用的
  不會自動回滾）。
- `safety-invariants-075`：L4「只這一次」目前是最短 TTL（5 分鐘），不是真正的單次授權（`Consent` 尚無
  `max_uses`／one-shot 欄位）。
- `character-protocol-043`／`safety-invariants-077`：外部 adapter 的輸入事件已完全不能合成人類互動觀察
  （比原本更嚴格），還沒有安全的新管道讓外部 adapter 未來能回報使用者互動。
- `interaction-api`：`adapter_token_http_routes_share_the_websocket_rate_limit` 這個真實時鐘限流測試在機器
  負載高（多個 agent 並行 build）時會兩邊 flake，單獨跑穩定通過；本輪未改動任何限流程式碼。

### Phase 8：Character Presentation Protocol、小樞改為 Reference Adapter、一般模式產品化（2026-09-02）

> 目標：Runtime 角色無關、呈現技術無關、AI Provider 無關、硬體無關。小樞不再是寫死的唯一角色，而是第一個完整的
> Reference Adapter；Runtime 只送語意化 Character Intent，呈現層沒有權限主權。唯一契約：`docs/character-protocol/README.md`。
> 測試數字見 `docs/acceptance-evidence.md` v0.5 Phase 8 章節（每個數字都是本機實跑；模擬器／fixture 一律標示）。

#### Added — Canonical Character Presentation Protocol 1.0
- **`crates/interaction-character`（新 crate，純函式、無 I/O）**：`CharacterManifest`（schemaVersion／characterId／displayName／
  adapterKind／entrypoint／assets／capabilities／inputCapabilities／channels／states／intents／variants／locales／pronouns／
  preferencesSchema／securityRequirements／resourceLimits／fallbacks／compatibility）＋ §2.1 驗證（大小、路徑穿越、magic bytes、
  builtin 白名單、preferencesSchema 白名單子集、安全錯誤訊息）＋舊 Character Pack v1／v1.1／rig 2.0 → manifest migration；
  26 個 canonical capability id＋namespaced custom；20 個 intent、15 個 truthState、priority floor（emergency 100 … AI 上限 50）；
  `IntentEnvelope`（messageId／correlationId／interruptPolicy／resumePolicy／durationHint／presentationHints／privacyClass／expiresAt）；
  13 種 input event＋正規化器（hover ≤4/s、drag ≤10/s＋8 px 量化、佇列 64、file-drop 只 metadata＋≤10 分鐘 grant、
  absolute 座標／原始路徑一律丟棄、observer／notification-only 不轉發）；10 種回執狀態與合法轉換（accepted≠started≠completed；
  acknowledged 永遠不會變 completed）；14 個 adapter 生命週期狀態；wire messages（hello／negotiate／negotiated／intent／cancel／
  heartbeat／error／goodbye／receipt／event／lifecycle）＋64 KB／50 msg/s／pending 64／outbound 32 上限；純狀態機 `Gateway`
  （能力協商 exact／substituted／reduced／unsupported／failed、去重環 256、過期不播、安全 intent 永不丟、搶占規則、
  generation／stale 拒絕、crash→uncertain、多 instance 安全去重、`system.text` 最後退路）。JSON Schema golden
  `schemas/character-protocol.schema.json`（由 Rust 產生）。
- **TypeScript 鏡射** `apps/interaction-desktop/src/character/`：protocol／manifest（驗證＋migration）／negotiate／adapter 介面／
  in-process `CharacterGateway`／registry（`public/characters/index.json`＋9 份內建 manifest）／`lines.ts`；reference adapters
  `text`（最小文字角色，也是可信 fallback）、`sprite`（舊 v1／v2 pack 相容層）、`shu`（小樞 rig）。
- **Runtime 接線** `crates/interaction-runtime/src/character.rs`：`CharacterHub`；真相投影（agent.session.state／action.*／
  plan.blocked／emergency／proactive／provider／receptor.observation／AI `state-present`→intent，AI 的 wait-attention／
  look-at-confirmation 自動換成 think／notice）；`character.intent`／`character.receipt`／`character.instance`／
  `character.system-text` 事件（agent token 看不到）；回執誠實結算 presentation receipt（completed→Completed AcknowledgedOnly、
  unsupported／failed→Failed、cancelled→Cancelled、expired／uncertain→Uncertain，永不 verified）；沒有任何 instance 時安全
  intent 走 `system.text`（不遺失）。
- **HTTP／WebSocket／CLI**：`POST /v1/character/{hello,receipts,events,adapters,intent}`、`GET /v1/character/{instances,manifest,
  adapters}`、`DELETE /v1/character/adapters/{id}`、`GET /v1/character/ws?token=`（只收 adapter token；human／agent token 401）；
  adapter token（sha256 儲存、撤銷即 goodbye＋斷線）只能打自己 instance 的 receipts／events 與 WS，打任何人類路由 403；
  `interact-ai character status|instances|manifest|adapters list|add|revoke|intent`（安全 intent 一律拒絕）；storage v8
  `character_adapters`；`status.characterProtocol`。外部 reference adapter：`examples/character-adapters/text-adapter.mjs`
  （Node ≥ 22、零依賴；CLI E2E 以「模擬 adapter（fixture）」閉環驗收）。
- **小樞 → Reference Adapter**：`ShuCharacterAdapter`＋`shuTables.ts`（intent→表情表、ambient／反應／落地／睡眠／個性權重
  全部搬進 adapter；`claim-completed`＝success-claimed，`verified-success` 只在 truthState verified 才是 success-verified）；
  `InteractionDirector` 引擎中立（表與 `isPlayable` 注入，不再 import rig）；`CompanionApp` 以 `CharacterGateway` 為中心
  （角色由 index＋prefs 選取、載入失敗／崩潰回退文字角色＋固定文案於可信 DOM 元素、`character.intent` 餵入、回執／輸入走
  `/v1/character/*`、舊 daemon 走 legacy 路徑）；`StageRenderer.pause/resume/setPalette`（隱藏真的停 rAF／物理）；
  小樞 36 表情與遊玩功能全部重跑通過。
- **可信 host 層（Tauri）**：`overlay` 視窗（透明、穿透、最上層）只在 estop／感測使用中／Runtime 離線時出現，內容只來自
  Rust `emit_to("overlay","host-safety")`（`host_safety.rs`；tray 共用同一份 view；非 macOS tray 文字也顯示感測）；
  main／companion 視窗 capability 移除 emit（renderer 無法偽造 host-safety）；`character_import/list_imported/asset/remove`
  （Rust 驗證＋magic bytes＋大小＋路徑再檢查，存 `<home>/state/characters/<id>/`，只允許 in-process 白名單 builtin）；
  `character_hello/receipt/event/instances/manifest/adapters/adapter_revoke` IPC（內嵌／外部 daemon 兩模式）。
- **一般模式狀態投影** `src/statusProjection.ts`：exhaustive（`satisfies Record<WorkState, …>`）的人話對照
  （準備中／處理中／等你補充／等你同意／無法繼續／Agent 說已完成，等待檢查／已確認完成／執行失敗／執行逾時／已到期／
  結果不確定／已停止）；未知原始值一律顯示「結果不確定」而非原始字串；AiPage／HomePage／Inbox／ActivityPage／GlobalSearch 共用。
- **CI（Linux）修復**：`interaction-adapter-declarative` 的 `link_transports_validate_identity_command_and_facts` 在沒有 BLE 的 Linux
  上以小寫 `"not supported"` 比對誠實拒絕訊息（實際是 `"NOT supported in this build"`），本機 macOS 走 UUID 分支所以看不到；
  斷言改對齊訊息（兩個平台都是合法結果）；CI 的 `cargo test --workspace` 加 `--no-fail-fast`，一次曝光所有失敗；以 Docker
  `rust:1.94-bookworm` 在本機重跑 Linux 的 fmt／clippy／test 確認。
- **工程修復**：`protocol.rs::wait_for` 改回傳 `WaitError{TimedOut{lagged},Closed,Lagged}`＋`LagPolicy`（握手 closed 誠實映成
  `LinkError::Reset`）；`rust-toolchain.toml`（1.94.0，CI 從此檔讀 channel）；三個對抗審查 workflow 不再硬編路徑
  （preflight 解析 git root、規格存 `docs/specs/`、v05 版輸出 `docs/reviews/adversarial/<runId>.{json,md}`、
  `.claude/workflows/README.md` 說明 runtime）；相容路由 `work↔automations`、`connect↔safety`、`memory↔activity↔settings`
  在已掛載元件上真的切換（`useEffect` 同步 `initial`＋`compat-routes.test.tsx`）。

#### Added — 一般模式產品化（五入口）
- **導覽跟著角色**：`src/characterName.ts`（`useCharacterName()`：prefs 名字 > manifest displayName > 「角色」；代詞來自 manifest，
  缺省中立）；第二個入口顯示目前角色名（預設小樞），更換角色或載入失敗後不再寫死。
- **現在**：第一屏只回答三件事——角色現在怎麼樣（含固定安全文字與「角色離線，改用文字」可信 fallback）、正在做什麼
  （人話狀態投影）、有什麼需要處理（收件匣）；快速操作「交代一件事／暫停或恢復主動互動／加入裝置」；Session／
  recipe／provider 數量收進「詳細狀態」。
- **角色**：目前角色（來源 內建／匯入／外部、能力摘要、即時狀態）→ 外觀與名字（variants 由 manifest）→ 平常如何陪伴
  （小樞保留安靜／自然／活潑 preset；其他角色由 `preferencesSchema` 產生 bounded 表單）→ 安靜與勿擾 → 更換或加入角色
  （內建＋匯入清單、匯入 manifest JSON、第三方／外部／可執行／需網路標示、停用回退文字角色、移除）；36 表情預覽只對
  `shu-rig` 顯示；rig 參數／channel／manifest JSON／engine id 只在進階「技術資料」。
- **工作**：任務優先——「想讓{角色}幫你做什麼？」＋加入檔案或選擇資料夾＋開始；建立前預覽使用哪個 Agent／讀取範圍／
  是否寫入／工具／時間、訊息與費用上限／如何取消；Agent 管理收進「工作設定」；claimed 只顯示「Agent 說已完成，等待檢查」
  ＋人工驗證按鈕，verified 才有綠勾。
- **連接與權限**：第一層四區——可以看見／可以回應／使用的裝置／需要你確認；角色 adapter 出現在裝置與整合來源
  （內建或第三方、本機或外部、是否有可執行程式、是否需要網路、可以接收哪些資料、是否已測試、撤銷）；
  完整 receptor／actuator／provider 收進「全部能力與裝置」。
- **更多**：記憶與知識／活動歷史／設定／角色與整合管理／進階功能（進階模式切換唯一主人）；匯出／還原／清除／原始 context
  預覽收進第二層。
- **首次成功體驗**：三步精靈之後可略過的「角色準備好了。要不要先試一次？」（提醒我休息＝本機提醒不啟動 Agent；
  交代一件小工作＝走 claim→人工驗證→verified；先在桌面陪我；更換角色；稍後再說）；角色不可用時用可信文字；390px 可完成。
- **一般模式術語清理**：Runtime／daemon／token／CLI／Provider／受器／動器／Lease／Receipt／JSON／YAML 只在進階模式或折疊的
  技術資料；Codex／Claude Code／USB Serial／Bluetooth LE 等產品名附用途說明。

#### Changed
- Presentation Provider id `provider.companion.shu` → **`provider.companion.desktop`**；顯示名跟著目前角色
  （「桌面角色：小樞（Presentation）」／未連線「桌面角色（尚未連線）」）。
- `PLAYABLE_ANIMATIONS` 常數移除：AI 可點播動畫＝協商到的 `visual.expression.variants` ∪ 非安全 canonical intent − 真相狀態
  deny-list；未 hello 前只有 9 個 canonical。iPhone `notify.show` 預設標題改用目前角色名（fallback「角色」）。
- Runtime 端節流：receptor.observation→notice 每 receptor 2 s 一次；drag 觀察 1/s；hover→pointer 30 s 一次。
- 硬體「已測試」證據文案改人話：「裝置報上身分並完成配對：感知來源 「<名稱>」（<id>） 讀取成功」（不再出現 受器／hello／pair-ok）。
- `DesktopPrefs.companionPreferences`（每角色 ≤32 鍵、布林／數字／字串 ≤200 字、≤16 角色）與 `UiPreferences.firstSuccessSeen` 新欄位；
  `GET /v1/character/instances`／`adapters` 多回報 `author`／`version`／`inputCapabilities`（adapters 另有 `characterDisplayName`／
  `adapterKind`／`executable`／`network`）；`character_list_imported` 帶完整 manifest。
- `companion-reload`：只改可即時套用的偏好時就地 `reconfigure`，不再整個視窗重載（換角色／persona／尺寸仍重載）。

#### Fixed — 對抗審查（run 1／2；報告在 `docs/reviews/adversarial/`）
- **AI 誠實階梯**：codex app-server `turn/completed` 讀 `turn.status`（interrupted→cancelled、failed→failed、缺 status→unknown，不再一律當 claim）；
  多輪 session 每輪重置 claim／result 旗標，第二輪子程序死掉會誠實報 unknown／failed；訊息未送達（上一輪還在跑／預算／stdin 逾時）
  回 409／403／503 且**不**把 session 記成 failed（只有 stdin 真的關閉才 failed）；人類驗證綁定具體 claim（`claimId`／
  `humanVerified.claimId`），新一輪任務或新 claim 會清除綠勾；看門狗自動拒絕送不到 agent 時保留 pending、回報 `deliveredToAgent:false`
  而不是假裝 progress；`close` 不再把 failed／unknown／timed-out 改寫成 closed。
- **收件匣誠實**：`ActivityInboxFilter.needsDecision` 篩選；通知中心與「需要你確認」在 `pendingCount` 大於本頁待決定數時顯示
  「還有 N 項待決定不在這一頁」而不是「沒有待決定事項」；安全事件標題改人話、解除緊急停止投影成「緊急停止已解除」。
- **記憶與知識頁**：Context Bundle 的 `needsReview` 是陣列，一般模式改正計數（原本算成 NaN 而永遠顯示「沒有」）；bundle 另回報
  未複審候選與領域外排除；角色互動記憶可「忘記這些」；頁面改用目前角色名；素材區依一般／進階分層；文案只宣稱真的記錄的項目。
- **工作頁**：「精靈選擇」改為「目前分工」（不冒充精靈選擇）；進行中判斷與 Rust `is_open` 一致（failed／unknown／timed-out 不再顯示續租／中斷）。
- **角色視窗**：幀預算改量真實繪製成本（原本量 rAF 間隔，60 Hz 螢幕一秒後永久降到 30 fps）；SpriteRenderer 有 pause／destroyed 旗標、
  Reduced Motion 只畫一次靜態幀；舊路徑 `agent.session.state` 的 unknown／expired 映射到「結果未知」；`presence-set` 真的顯示／隱藏視窗
  （新 Tauri 命令 `companion_set_visible`），隱藏兩條路徑都通知 runtime presence；file-drop 事件 Rust 正規化器同時接受 `files:[…]`，
  Runtime 觀察回報全部檔案。
- **裝置線協定**：韌體 BLE 廣播加 scan response 名稱；模擬器加控制通道（`--facts-file`／按鈕翻轉／`--sensors-absent`）並在 CLI E2E
  驗證未請求的 state 推播與 null 感測；estop stop-all 等待拉到 2 s；host 端 serial／MQTT 送出前檢查 639 bytes 上限（超過誠實
  Refused，不製造未知）；`buzzActive` 三端一致；配對失敗 5 次鎖定 30 s（hello `pairingLocked`）；README nonce 敘述修正。
- **效能宣稱**：8.3 ms 明確標為 WebView 內段，端到端未量；`pnpm perf` 加 `--enable-precise-memory-info` 與 stage loop 統計；CHANGELOG hit-rect 敘述修正。

#### 已知限制（Phase 8；完整清單與證據見 `docs/acceptance-evidence.md`）
- **真機／真視窗證據為零**：Tauri 角色視窗、可信 overlay 視窗、匯入角色資料夾、外部 adapter 都只有單元／模擬器／fixture 證據；
  Playwright 只覆蓋瀏覽器版控制中心；ESP32／iPhone 真機仍未驗收。
- stdio JSON Lines transport 只有規格，沒有 host spawn 也沒有 fixture（外部 fixture 走 WebSocket）；`entrypoint.process` 只是紀錄。
- 匯入只接受 in-process 白名單 builtin（`shu-rig`／`sprite`／`text`）與純資料資產；沒有簽章機制（UI 一律標示「簽章：無」）；
  Live2D／Spine／Rive／3D／影片等引擎沒有內建 adapter——它們要以外部 WebSocket adapter 或未來的 in-process adapter 接上。
- 外部 adapter 一律以 `familiar` 角色加入，還沒有 UI 可指定 role；多角色安全去重只按 role class。
- `hello` body 上限 256 KB，最大 manifest 加 negotiate 副本可能超過（極端情況，未處理）。
- ~~工作頁「選擇資料夾…」在桌面版沒有內建資料夾選擇器（無 dialog plugin），以路徑文字欄代替。~~
  已在 Phase 9 解決（`tauri-plugin-dialog`，僅 `main` 視窗；瀏覽器版誠實顯示不支援），但沒有自動化驗收，
  見 Phase 9 已知限制。
- iPhone `character.present` 仍是獨立的 7 態 actuator，未改走 CPP（「前往 iPhone」呈現表面未做）。
- 效能數字仍為 headless Chromium（`pnpm perf`），非 WKWebView。
- 對抗審查未修（誠實記錄）：多角色「玩耍」只有使魔↔使魔與使魔→主角 greet（使魔不追球、主角不主動找使魔）；頭飾光沒有真實連線
  狀態輸入（只由表情 hold 寫入）、奔跑不歪頭飾、袖口面板與口袋取物未實作；36 正式表情四段全手寫只有 4/36（enter 20／loop 29／exit 11
  手寫，其餘派生並標示）；Attention／Utility scoring 在生產路徑只在同優先平手時以常數情境呼叫（實際為死碼）；Gateway session 訊息未送達
  （上一輪還在跑／預算／stdin 逾時）時訊息仍留在信箱且 `deliveredAt` 為 null、佔一則預算，UI 尚未提供「重送」；輸入延遲只量 WebView 內段
  （8.3 ms），端到端（OS 指標→點擊穿透→toy.grabbed）未量，不得宣稱達到 16–100 ms；所有 agent-honesty 修復皆以 fake 子程序驗證，真 codex／
  claude 二進位未跑。

### Phase 7：整合、對抗審查、修復、全套回歸（2026-08-28）

> 這一段記錄的是「把 Phase 1–6 宣稱的東西變成真的」：新 Session 先不信任前一輪的進度敘述，
> 用 10 位審計 agent 逐條對照規格重建恢復矩陣（`docs/v05-recovery-matrix.md`），再跑
> `.claude/workflows/adversarial-review-v05.js`（11 維度 find → 獨立懷疑者 verify），
> 136 項審查 → 73 confirmed／59 fixed-meanwhile／4 refuted，只修 confirmed，每項附 regression test。
> 測試數字：Rust workspace **349 → 426**、vitest **138 → 319**、CLI E2E **59 → 63**、Playwright 24、Tauri 4 → 8、
> iOS XCTest 19（模擬器）。模擬器復測：撤銷 ≤0.035 s 斷線、estop 0.5 s 內停手機感測、Bonjour 真的廣播。

#### Fixed — 誠實階梯與安全底線
- Agent 程序結束而無結果 → 新 taxonomy 狀態 **`unknown`**（不再謊報 failed）；lease 到期／重啟發 timed-out／unknown；
  Claude `system/init` 不再等於 working（第一個 assistant/tool 事件才算）；codex／claude 的 `fetched` 只在 stdin 真的
  write+flush 後才發；approval 裁決（人類或 300s 看門狗）回寫信箱 `approval-resolved`，AiPage 顯示「已由看門狗自動拒絕」；
  人類 GET 信箱是「觀看」不是「送達」（不再冒充 delivered）；`PendingApproval.summary` 真的顯示；CLI `agents create --resume`／
  `agents resume`；codex `thread/resume` 真的重新上鎖 cwd／approvalPolicy／sandbox（以 codex-cli 0.150.1 產生的 schema 核對）。
- SSE：初次連線不再重播整個 ring buffer（改從 `status.eventSequence` 起）；daemon 重啟（`startedAt` 改變）重置 cursor；
  角色 machine 忽略早於 App 啟動的重播事件（safety 狀態仍以 `/v1/status` 為準）。
- iPhone：**撤銷即斷線**（CancellationToken；之前只移除表項）；ack/err/ble.* 需 authed 且綁定裝置；heartbeat 15s／idle 45s；
  斷線強制停用高風險 receptor 且重連不恢復；facts 依 manifest 過濾、mic-level 不持久化；**estop 同時停 iPhone 感測**
  （`stop-all{sensors:true}`，iOS 端同步）；啟用中的手機麥克風出現在 `status.activeSensors`（tray／首頁／角色視窗）；
  `iphone.mic-level` 需 session consent，撤銷即丟棄；**綠勾（verified-success）只走人類驗證路徑**，plan/agent 帶入一律拒絕；
  extra 參數不能放寬 policy 的 L3 硬限制；estop 500ms 內只送一則 stop-all、無連線誠實 Err；撤銷持久化失敗誠實回 Err；
  Bonjour 服務型別改 `_interact-ai._tcp`（舊名超過 15 bytes 註冊失敗）並在 status 回報 `bonjour`；`started` 只在 bind
  成功後設；devices 為空不開埠；`mobile ble-scan` CLI／Tauri／UI 可達；MobileSection 顯示手機 permissions 與 Bonjour 狀態；
  **測試模式不把 Bonjour 記錄廣播到實體區網**（模擬不得有外部副作用）。
- 硬體 link：health()/status() 依真實連線（斷線 offline、未握手 degraded，不再硬編 healthy）；serial ENOTTY 判斷收窄
  （實測 pty 錯誤字串）；MQTT dedupe／重連／QoS1 真斷言；provider 停用／撤銷關閉連線（serial/mqtt/ble `shutdown`）；
  `hello.caps` 能力識別（未宣告 → 不上線、failed）；hello `proto`／`pairing` 核對；state 核對 deviceId；cmd 帶 deadline、
  重連清佇列並以 `link-reset` 結束等待；無 id 的 err 以拒絕原因結束（不演逾時）；裝置 not-paired → 該次 failed、下一次前重握手
  （絕不自動重送實體命令）；BLE 途中失敗＝uncertain 不重試；stop-all 無 ack 誠實回 Err；握手逾時不再標 dispatched。
- L0 純呈現（`builtin.presentation`）的 uncertain 不進「待我決定」；外部裝置（含 iphone.character）仍進；Inbox pendingCount
  在截斷前計算；風險分級 L0–L4 標籤（`riskTier.ts`）；精靈預設不靜默寫入安靜時段／initiative，步驟二選擇真的寫入 agent 路由。

#### Fixed — 角色
- 四段式：timeline 真的播 **exit**（≤260ms；安全狀態立即搶佔不播）、缺段以派生段補齊並標 `derived`、8 個高頻表情手寫 exit；
  組合通道：工作／等待只覆蓋核心／頭飾／裙光／耳朵、身體保留遊玩姿勢；`personality.ts` 個性模型接進 director／playfield／timeline；
  `director.react()`／`noteFinished()`／`scoreEvent` 接進 App，quiet 分支可達，「一小時內不要主動說話」也管角色端；
  machine 旗標即時同步舞台（estop 不再多追 ≤480ms 球）；hit-rect 逐幀節流回報、Rust 點擊穿透輪詢 80ms；拖曳期間持續 hold；
  放下依速度／高度／落點四種落地；第 6 種玩具 trinket；多角色互看／回愛心／被追者逃跑；hover 短氣泡；硬體／提供者事件演出；
  氣泡／音效（預設關）／拖曳／勿擾開關；estop 停語音並清 transient；睡眠類 ambient 剛互動後不秒回；Reduced Motion 真靜態
  且執行中可切換；30fps 降級遲滯；`ask` 為真相狀態（AI 只能點播 `question`）；clampParams 嚴格型別；lerpParams 允許回彈；
  `poseBlend` 通道消除 lie↔stand 頭部瞬移；角色互動記憶（有界、不推論人格、不進知識庫）；Rust hit-rect clamp；隱藏視窗通知 runtime。

#### Fixed — 控制中心、記憶 UI、文件
- 通知中心鍵盤／焦點陷阱；淺色主題 `--panel`／`--input-bg`；一般模式術語外洩（Lease／provider session／raw JSON／UUID）改人話；
  §11 記憶與知識一般模式只有三區、規格人類文案、技術 tab 進階才顯示；到期記憶只能刪除（不再有必失敗的按鈕）；
  GlobalSearch 人話；AiPage 訊息 5 秒輪詢＋approval 倒數；WorkPage 顯示精靈選擇；守門測試（5 入口／單一主人／舊 tab 對照）。
- Provider「已測試」：`detail.tested` 證據（handshake／capability／human）、`POST /v1/providers/{id}/test`（human-only、唯讀）、
  CLI `providers test`、UI 六階人話＋「測試裝置」。
- 效能量測可重現（`pnpm perf`）：drawRig／全舞台／輸入延遲（真狀態改變）／heap／bounded／3 天數值；作廢無來源的舊數字。
- ESP32 韌體：arduino-cli 實際編譯兩組態（`compile.sh`＋Apple Silicon ctags shim）；浮點參數規則明確（兩端一致）；
  MQTT 非阻塞退避、效果進行中不連線；BLE 有界佇列由 loop 統一處理；nonce 環；模擬器鏡射 8 周邊與所有 err 形狀。
- iOS：對 iOS 26.5 SDK typecheck 0 error 0 warning；swiftc 直接編模擬器 .app＋DEBUG 啟動參數；與真 daemon 完成配對／動器／撤銷
  閉環；XCTest 在模擬器 19/19；`stop-all{sensors}` 停感測並顯示原因。
- 文件：README／CLAUDE.md／DESKTOP-GUIDE／FEATURES／acceptance-evidence／gap matrix 的過期與不實敘述全部改正
  （版本號、舊 IA、127.0.0.1 例外、「無 Xcode」、「撤銷立即斷線」、「36 表情皆三段」、HMAC 配對、看向游標、效能數字、59 checks）。

#### 已知限制（v0.5，完整清單見 `docs/acceptance-evidence.md` v0.5 章節）
ESP32 與 iPhone 皆無真機驗收；BLE 無真機；硬體 Observed/Verified 死路（acknowledged 為誠實上限）；provider 停用後連線關閉不可逆；
MQTT 內部佇列 deadline 伸不進；`waiting-input` 無 connector 來源；跨視窗桌面漫遊／邊緣探頭／其他視窗事件反應／Fullscreen／
OS 勿擾偵測未做；桌面 BLE gateway 只有 scan；Camera／Location／Live Activity／SFX／區網裝置事件未做；WebSocket／HID／
Home Assistant adapter 未做；配對期可被區網 peer 燒掉（已 audit）；效能數字為 headless Chromium 非 WKWebView。


### Added — Phase 2：正式小樞（Q 版貓娘女僕）與動畫核心

- **小樞 v3「女僕正式版」**：執行期參數化分層 rig（`companion/rig/`），
  非 sprite sheet——每幀由 `drawRig(params, palette)` 純函式即時繪製。
  約 2.5–2.6 頭身 Q 版：柔黑深灰紫短髮＋不對稱髮束＋呆毛、紫灰大眼、
  小虎牙；奶白×深灰紫女僕工作服（泡泡袖、工具圍裙＋口袋、蓬裙＋
  燈籠褲、圓頭軟靴、分體頭飾讓位貓耳）。3 個調色盤變體
  （classic／dusk／sakura），pack kind=`character-rig` schema 2.0。
- **服裝參與功能呈現**：左耳冷藍=感知、右耳暖橙=行動、胸前蝴蝶結
  結晶核心=Runtime/AI 工作（呼吸發光）、頭飾光=Agent 連線、
  裙擺細光=waiting（琥珀）/unknown（紫）/blocked（紅）、尾尖紫光=工具、
  policy 小盾保留。
- **組合式角色通道**：~40 個有界參數（body/head/gaze/eyes/brows/mouth/
  ears/hair/headpiece/core/arms/tail/legs/skirt-light/overlay/particles），
  `clampParams`/`lerpParams` 保證任何輸入都畫得出合法畫面。
- **36 個正式表情**（spec §7.5 全清單）：Phase 2 交付時只有 hold＋部分
  enter/loop（13/36 兩段齊全、0/36 四段齊全，「離開」段是死資料——Phase 7 對抗
  審查確認並修正為真四段式，見下方 Phase 7）；加基態/相容別名共 55 個表情；
  Game feel：anticipation、squash&stretch、hit-stop、粒子（灰塵/星光/
  愛心/zzz）、headpiece 歪掉扶正等 secondary motion。
- **RigRenderer**：表情 crossfade（~180ms 輕微回彈）、自動眨眼
  （幾何分布間隔）、gaze/ear 微動作疊加（僅 ambient 表情）、Reduced
  Motion 靜態姿勢仍保留狀態辨識；誠實映射：machine 的 success＋
  frameSlice（未驗證）→「聲稱完成」只點頭，無 slice（已驗證）→
  「驗證成功」才有綠勾與慶祝。truth-state 表情（成功/失敗/阻擋/未知/
  緊急/離線）不可被 AI 或 ambient 點播，fallback 鏈永不落到成功。
- **Interaction Director**（`companion/director.ts`）：統一行為導演——
  ambient 變體池（hazard 抽樣、每表情冷卻、防重複 3、放鬆度門檻）、
  被真實事件搶佔後可恢復（20 秒 TTL）、quiet 只剩眨眼、Reduced Motion
  只允許眨眼類、utility 評分沿用 EventClass 階梯；取代舊
  scheduleMicroAction 路徑。（Phase 2 交付時 `react()`/`noteFinished()`/
  `scoreEvent()` 尚未接進 App、quietHours 鍵在 runtime status 缺失——
  Phase 7 修正，見下方。）
- 控制中心小樞頁：36 表情即時預覽（與桌面同一套 rig 程式）；
  首次設定精靈的角色預覽同源。預設 pack 改為 `shu-maid`；
  v1/v2 sprite packs 保留為相容層（fallback 鏈不變）。
- 設計稿與驗收畫面：`docs/assets/v05-evidence/shu-maid-rig-sheet.png`
  （代表性姿勢×明暗底×3 配色＋36 表情 hold 網格）。

### Added — Phase 6：iPhone Mobile Provider

- **桌面端 Mobile 伺服器**（`interaction-runtime/src/mobile.rs`）：
  TLS WebSocket（自簽憑證、SHA-256 指紋由 QR 載荷釘選）＋Bonjour 廣播
  （服務型別 `_interact-ai._tcp`——Phase 6 交付時的 `_interact-ai-mobile`
  超過 RFC 6763 15-byte 上限、註冊靜默失敗，Phase 7 修正；TXT 帶指紋）＋
  配對儀式（一次性 6 位配對碼 5 分鐘、HMAC-SHA256 challenge-response、
  錯一次即作廢防暴力）＋每台 iPhone 獨立 device token（只存 SHA-256）。
  撤銷立即斷線與「已配對過才開網路埠」在 Phase 6 交付時**不成立**
  （撤銷只移除表項、連線仍活著；devices.json 存在即開埠），Phase 7 修正並以
  測試釘死。伺服器綁 0.0.0.0（區網），這是 iPhone 配對的刻意設計。
- **iPhone 能力接進既有管線**：4 個受器（motion 語意事件/battery/touch/
  mic-level——mic 需 consent）＋6 個動器（haptic/notify/tts/torch/flash/
  character，全部 consent-gated、健康度跟連線走）；act→ack 收據
  acknowledged＋deviceApplied；ack 逾時＝結果未知不重送；estop →
  stop-all 廣播到所有手機；斷線 → provider Disconnected、能力
  unavailable。「高風險感測不自動恢復」在 Phase 6 只由手機端強制，桌面端
  於 Phase 7 補上（斷線即停用 iphone.mic-level）。觀察 receptor 白名單
  ——Phase 6 只驗 receptor id，Phase 7 起依 manifest 欄位過濾 facts。
- **BLE Gateway 通道**：桌面端只實作 `ble.scan`（`POST /v1/mobile/ble/scan`、
  CLI `mobile ble-scan`）；iOS App 端有 connect/gatt(read/write/subscribe)
  實作但桌面端**沒有對應送端**——列為已知限制。
- **UI／CLI**：連接與權限→裝置與能力新增 iPhone 區（配對 QR SVG＋配對碼、
  裝置清單含連線狀態與手機自報感測、撤銷）；CLI `interact-ai mobile
  status|pair|revoke`；`/v1/mobile/*` human-only（agent token 連 GET 都拒）。
- **iOS Companion App 原始碼**（`apps/interaction-ios/`，SwiftUI）：
  配對（QR/手動＋憑證指紋釘選）、語意 motion 分類器（lifted/shaken/
  placed/rotated，3 秒視窗記憶體內、不存軌跡）、感測預設全關＋權限誠實
  顯示＋斷線自動停用高風險感測、haptic（含 purr/heartbeat）/通知/TTS/
  手電筒/螢幕閃示/角色狀態（綠勾只在 verified-success）、CoreBluetooth
  gateway。Phase 6 交付時只做了 `swiftc -parse`（當時誤以為本機無 Xcode）；
  Phase 7 以 Xcode 26.6／iOS 26.5 SDK 完整 typecheck、編成模擬器 .app 並與真
  daemon 完成配對閉環（見 Phase 7）。**真機未驗收；不宣稱 iPhone 可操作任意
  USB 配件。**
- 測試（【模擬 iPhone】程序內 client，明確標示非真機）：Phase 6 交付 3 個
  測試（TLS 指紋釘選配對閉環、錯誤配對碼拒絕＋配對期作廢、token 重連／撤銷後
  auth-fail）；「觀察 ingest＋白名單」「斷線→Disconnected」當時只有送出沒有
  斷言——Phase 7 補齊為 16 個測試（含 mutation 驗證）。

### Added — Phase 5：真實硬體（Serial／MQTT／BLE adapters＋ESP32 參考裝置）

- **裝置線協定 v1**（`interaction-adapter-declarative/src/protocol.rs`，
  三傳輸共用）：hello 身分驗證（**埠/IP/topic 永遠不是身分**——
  hello.deviceId 必須等於 spec.expectedDeviceId，不符即拒）、配對碼握手
  （pair/pair-ok/pair-fail，建議 secret://）、cmd 帶 action id＋nonce、
  裝置端 16 筆 id ring dedupe（重複 ack dup:true 不重放效果）、cancel、
  read/state、stop-all。**ack 逾時＝結果未知且絕不自動重送**（實體效果
  不得重複觸發）；斷線退避重連（1s→15s cap）後強制重新握手。
- **Serial 傳輸**（feature transport-serial；serialport crate，macOS pty
  模擬器 ENOTTY 時誠實退回純檔案 I/O）＋**MQTT 傳輸**（transport-mqtt；
  rumqttc，QoS1 `<prefix>/to-device|from-device`）＋**BLE 傳輸**
  （transport-ble；btleplug GATT command/state characteristics，僅
  macOS/Windows 編譯——Linux 誠實拒絕）。收據誠實：send 失敗=failed、
  裝置 ack=acknowledged（附 deviceApplied 揭露韌體 clamp）、ack 逾時=
  dispatched+ackTimeout→runtime watchdog 標 uncertain；estop 對每台
  裝置直送 stop-all。
- **YAML spec 擴充**（schema 追加欄位）：`request` 改為選填（http/sse
  專用）；link 傳輸用 `command:{name,params}`＋`serial:/mqtt:/ble:` 設定
  塊；同一 adapter 的 link capability 必須指向同一台裝置。
- **ESP32 官方參考裝置**（`firmware/esp32-companion/`）：Arduino 韌體
  （USB Serial＋Wi-Fi/MQTT；BLE 可選 NimBLE）、RGB LED/按鈕/HC-SR04/
  光敏/DHT22/震動馬達/伺服/蜂鳴器；**韌體硬限制**（vibe duty≤0.8、
  ≤3000ms、脈衝間隔≥500ms；buzzer 200–4000Hz、≤2000ms、duty≤50%；
  servo 10–170°、300ms 節流）主機不可解除，clamp 後以 ack.applied 誠實
  回報；BOM／接線圖／Flash／測試步驟／YAML 範例齊備（zh-TW README）。
- **模擬器與真機分離標示**：`scripts/esp32-serial-sim.py`（pty，與韌體
  同協定、模擬硬限制與 dedupe）；CLI E2E 新增「Serial hardware closed
  loop (SIMULATOR)」段——provider 註冊→三段式人類授權（enable→policy
  allowlist→consent）→受器經配對握手讀 facts→cmd ack acknowledged→
  deviceApplied 揭露 1.0→0.8 clamp→estop stop-all 送達裝置（Serial 段 4 個
  check；整支腳本當時共 59 checks，Phase 7 增為 63）。
  MQTT 閉環以內嵌 rumqttd broker＋模擬裝置測試（含身分不符拒絕）。
  **真實 ESP32 硬體驗收未執行**（本環境無實體裝置）——Phase 5 交付時韌體僅經
  程式碼審閱＋協定模擬器對測；Phase 7 已用 arduino-cli 對 esp32:esp32 3.3.11
  實際編譯兩種組態（`firmware/esp32-companion/compile.sh`），仍未在真板驗收。
- 裝置 UI（連接與權限→裝置與提供者）：provider 卡片顯示人話狀態說明
  （只發現≠已配對≠已啟用；「已測試」於 Phase 7 補上）＋「小樞可以知道／
  小樞可以做」裝置導向能力清單。

### Added — Phase 4：AI 角色閉環（taxonomy、人工驗證、對話層）

- **Agent Session taxonomy 事件**（`agent.session.state`）：runtime 對
  created（queued）/fetched（任務真的送進子程序）/working/waiting-input/
  waiting-consent/claimed-completed/verified/failed/timed-out/cancelled/
  closed 發出標準化事件；小樞逐一映射成演出——等 Codex/等 Claude 專屬
  等待動畫、fetched=翻找、working=努力工作、waiting=請求確認、
  claimed 只點頭、cancelled 誠實清場。
- **人工驗證（claim → verified 唯一路徑）**：
  `POST /v1/agent-sessions/{id}/verify`（human token 專屬；agent/session
  token 一律 403）、CLI `interact-ai agents verify <id> --note`、
  AiPage「標記為已驗證（我確認過結果）」按鈕。記錄
  `humanVerified{at,note}`（audit 留痕）；只有 verified 事件會讓小樞播
  綠勾與慶祝。不可重複驗證、active session 不可驗證、備註 ≤500 字。
- **Resume 通路打通**：`CreateAgentSession.resumeProviderSessionId` →
  gateway SessionSpec（claude --resume / codex thread resume；sandbox 與
  權限旗標由 connector 重新上鎖，不繼承不放寬）；AiPage 已關閉 session
  提供「接續上次（唯讀）」。
- **Conversation Provider 介面（L1）**＋本機降級：
  `companion/conversation.ts` 可插拔介面；預設 LocalTemplateProvider
  純確定性規則——決定是否回話、選語氣與 behaviorIntent、判斷是否建議
  建立 Codex/Claude 任務；問句誠實承認「本機沒有答案」，絕不為普通
  反應啟動昂貴工作 Agent。
- 誠實不變量回歸釘死：runtime 測試（verify 唯一路徑/不可重複/長度界線）、
  前端測試（claimed→success-claimed 無綠勾、verified→success-verified）、
  CLI E2E（+3 verify 檢查）。

### Added — Phase 3：遊戲互動（VS Code Pets 基準＋超越）

- **遊玩場（StageRenderer）**：小樞的透明視窗加寬為小舞台
  （視窗＝角色寬×2.6、高×1.35），角色可在場內散步、翻面、追逐；
  hit-rect 隨角色與玩具動態更新（≤60 ms 節流回報給 Rust；視窗隱藏／rAF 被節流時以 500 ms 心跳後備），
  空白區維持點擊穿透。
- **第一批玩具＋輕量 2D 物理**（`companion/playfield.ts`，純函式可測）：
  毛球/紙團/紙飛機（滑翔）/光點/逗貓棒；資料模型含位置/速度/重力/
  碰撞/抓取狀態/擁有者/興趣值/冷卻/生命週期（150s TTL、上限 4）。
  拖曳投擲：方向與速度來自實際拖曳軌跡；牆壁與地面反彈、摩擦停下。
- **追逐/撲抓/帶回/拒絕歸還**：hazard 抽樣（dt 縮放，非固定週期）；
  撲抓 75% 成功（光點永遠撲空）；30% 機率想獨占（keep-ball 演出後
  才放下）；玩過的玩具進冷卻。machine 真實事件（點擊/工作/安全狀態）
  永遠搶占遊玩；緊急/離線/暫停凍結全場。
- **多角色架構（小型使魔）**：同一遊玩場最多 3 隻迷你貓精靈，
  自主散步/睡覺/互相注意/打招呼（愛心）/追逐；可個別命名、換配色、
  移除——與 VS Code Pets 同為單一面板多寵物模型。
- **場景**：透明桌面（預設）/桌面巢穴/工作桌/窗台/夜間——低調小道具，
  透明模式完整保留。**Roll Call**：「現在大家在做什麼」人話清單，
  由角色視窗經 presentationHello 誠實回報（Rust 端白名單＋長度驗證），
  控制中心小樞頁顯示；machine 真實狀態（等待確認/緊急停止…）優先。
- **拖曳體感**：拖起小樞＝lifted 懸空演出；放下＝wobbly-landing
  （灰塵粒子＋頭飾歪掉再扶正）。連戳 ≥3 次＝poked-rapid 抗議、
  不再狂開關選單。
- **玩耍偏好（單一主人＝小樞頁）**：名字、場景、玩耍/游標互動/靠近
  反應/自主散步各自可關；角色設定匯出/匯入（白名單驗證，不含權限/
  位置/token）。游標座標只活在角色視窗 canvas 內，永不送 runtime/AI、
  不持久化；Reduced Motion 自動停止玩耍與移動、玩具不彈跳。

### Changed — Phase 1：控制中心簡化

- **一級導覽 9 → 5**：現在／小樞／工作／連接與權限／更多。
  工作＝AI 工作階段＋自動互動；連接與權限＝裝置與能力＋同意與安全；
  更多＝記憶與知識＋活動歷史＋設定。舊 tab id（tray 深連結、Runtime Inbox
  route：`ai`/`automations`/`capabilities`/`safety`/`memory`/`activity`/`settings`/
  `senses`/`responses`/`toolops`）全部相容折疊到新入口，不破壞既有深連結。
- **Activity 改為右上 Inbox**：待決定事項在右上角「通知中心」逐項可前往；
  完整活動歷史移到「更多 → 活動歷史」，不再佔一級入口。
- **首頁瘦身（現在）**：移除完整權限地圖（唯一的家改為「連接與權限 → 同意與安全」）；
  保留系統狀態、感測／待決定／進行中工作摘要、最近互動故事與快速操作。
- **每項設定只有一個主人**：小樞外觀／Persona／劇情進度／主動式對話／
  AI 主動程度／安靜時段全部集中「小樞」頁；設定頁與同意與安全頁只留摘要與
  「前往小樞」，不再有第二份相同開關。
- **首次設定精靈 7 步 → 3 步**（認識小樞／AI 幫手／安全預設）：低風險本機能力
  自動保守挑選（未知不預選不變）；AI 幫手只做 discovery／登入檢查，不授權工作區；
  主動對話預設「必要時」；硬體掃描移出精靈；仍走同一 onboardingCommit 原子契約。
- 進階模式技術頁面全部保留（零能力退化）；緊急停止入口不變
  （頂欄／tray／全域搜尋／CLI，解除仍走同意與安全的安全流程）。

## [0.4.1] - 2026-08-28

### Fixed

- Keep macOS hardware-profiler helpers out of non-macOS production builds while retaining
  cross-platform fixture tests, and adopt Rust 1.98-compatible Clippy forms for activity
  ordering, knowledge pagination, and WAV sample parsing.

## [0.4.0] - 2026-08-28

### Added — v0.4：Presentation Provider、真實 Agent Connector、記憶與知識系統、控制中心新 IA

- **Presentation Provider**（`provider.companion.shu`）：桌面角色正式成為一級 provider，
  能力**逐項**宣告——7 個語意 receptor（點擊／文字輸入／快捷／拖放／游標語意／動畫事件／
  氣泡事件）＋7 個 actuator（狀態呈現／播放動畫／氣泡／音效／語音／視窗調整／顯示隱藏）。
  誠實迴路：`presentation.command` SSE → 視窗渲染 → ack → receipt
  （Dispatched→Acknowledged→Completed，證據誠實標 AcknowledgedOnly；10 秒無 ack →
  Uncertain）。音效／語音／視窗調整／顯示隱藏 consent-gated 預設停用；隱藏角色時視窗內
  receptor 確定性拒絕 ingest（隱藏≠緊急停止）。behaviorIntent／動畫白名單 runtime 驗證，
  成功／阻擋／緊急等真相狀態 AI 不可點播。
- **小樞 v2 貓系重設計**：3 頭身 Q 版貓系數位小精靈原創 rig——眼尾上揚狡黠眼＋逐眼眉毛
  眼皮（「發現了」「真的假的」「讓我看看」）、貓耳=注意力顯示器（earPerk）、尾巴重量與捲曲、
  23 個動畫（察覺反應鏈：耳先動→眼亮→頭後轉；失敗專屬美術＝愣住→認真檢查＋✕；
  伸展／晃腳／抱尾巴／趴下慵懶反差）。三變體共用骨架：靈巧（預設）／慵懶／活潑。
  成功綠勾契約不變（幀 0-1 點頭、幀 2-3 才有勾）。v1 packs 保留相容。
- **Behavior Runtime**：純本機確定性三層生命系統（不用生成式 AI 逐幀控制）——
  BehaviorState 平滑量（activation／attention／taskLoad／readiness／familiarity）、
  Utility AI 優先階梯（緊急>感測安全>等待確認>直接互動>任務>建議>世界觀>待機）、
  hazard 抽樣微動作排程（幾何分布間隔、反重複、放鬆度門檻、Reduced Motion 只留眨眼、
  seeded RNG 可重現）＋程序化視線／耳朵疊加（安全／Reduced Motion 狀態凍結）。
- **主動式對話政策**：off／necessary／natural（預設）／lively／custom 五模式，Rust 確定性
  強制——每小時上限 3、最短間隔 12 分、30 秒合併、未回覆不追問、勿擾延後、安全類只去重
  永不被壓制；狀態跨重啟持續。氣泡快捷「一小時不要說話」「今天安靜一點」。生成式觸發
  會建立真實、有期限/訊息/費用範圍的本機 Agent Session，只呈現通過 schema 的候選。
- **Agent Gateway（無模型 API）**：直接連本機已登入的 **Codex**（`codex app-server`
  stdio JSON-RPC，協定 schema 由 `generate-json-schema` 鎖定；不支援時降級至
  `codex exec --json`／`exec resume`；sandbox=read-only、approval 預設拒絕、逾時
  自動 deny）與 **Claude Code**（`claude -p` stream-json；預設 plan-only，經明確
  workdir＋二次確認才開限權 `acceptEdits`，絕不用 skip-permissions）。claims 走 v0.3 誠實路徑
  （inference 0.5、claimActionId 防偽）；成本入 SessionBudget 硬上限；estop／close／
  lease 到期終止整棵子程序樹；子程序不跨重啟存活；不讀取任何 credential。
  確定性路由建議（程式→Codex、文件→Claude Code、模糊→列兩者）。
- **記憶分層（10 層）＋保存期限三態**：expiresAt（到期停用並刪）／reviewAfter（stale 需
  重新確認）／until-deleted（不存在不可刪的永久記憶）。agent 寫入降權（fact→inference、
  長期使用者記憶→candidate＋30 天複查）；secret 樣態拒收；到期 watchdog 清除；
  **確定性 Context Bundle**（stale／敏感／denylist／candidate 排除並揭露）；每個真 task
  自動附上 Session/Domain 授權的最小 bundle 並持久化實際提供證據。JSON 備份可逐筆經
  human-only Runtime 驗證還原，不 raw overwrite SQLite。
- **多模態素材庫（CAS）＋知識圖譜**：SHA-256 內容定址 write-once 素材（AI 不可覆寫／
  刪除來源；刪除有影響預覽，失去來源的 Active 知識標 disputed 不靜默消失）；
  Entity／Claim／Source 節點＋12 種關係型別＋認識論來源標記（research-supported／
  analogy／ai-conjecture／user-confirmed…）；claim 必附證據（素材 hash＋片段語法）；
  類比／推測不可標因果；candidate→active→stale→disputed→superseded→archived 狀態機，
  只有 active 參與回答；FTS5（bm25）＋本機 sparse subword embedding（可重現、
  可替換，不宣稱 neural embedding）。縮圖與 WAV features 有 production pipeline；
  OCR/whisper/ffmpeg 本機工具缺少時回報 unavailable。
  9 個 `interaction.knowledge_*` canonical tools：AI 讀受限、寫一律 Candidate、
  agent 裁決降留言，activate 只屬人類。
- **知識更新決策器＋經驗轉知識＋Knowledge Receipt**：「要不要更新」與「要不要 AI」分開
  確定性判斷；外部研究必先問；發布三級政策（auto metadata／candidate-only AI 內容／
  必須人類確認）；freshness sweep 標 stale、衝突雙方標 disputed；任務結束確定性收集
  TaskMemory，學習訊號才建 Reflection Candidate，升格必須補反例＋適用範圍；
  每次知識變化寫機器可讀 receipt（誠實 conflictCheck 三態＋humanReviewed）＋
  `knowledge.updated` 事件。控制中心加入 update-check 與「使用者糾正」入口；糾正只建立
  30 天複查 UserMemory＋Knowledge Candidate。小樞把 receipt 映射為六種固定安全文案。
- **metadata-only 硬體掃描**：跨平台 discovery adapter 介面＋17 類覆蓋報告；掃描不開
  攝影機／麥克風／HID／BLE／mDNS。Linux 只讀 `/dev/*/by-id` 穩定連結；macOS 用
  `system_profiler` 列舉 camera/mic/audio/display/USB HID/controller/serial/Bluetooth/MIDI，
  volatile path 不冒充永久身分；不可用／需權限／未支援皆附原因。首次設定、控制中心、
  API `POST /v1/hardware/scan` 與 CLI `providers scan` 共用同一 Runtime Truth。
- **API 身分分權**：human token（`state/api-token`）、restricted agent token
  （`state/api-agent-token`）分離、constant-time 驗證。agent token 可以讀狀態、呼叫
  canonical tools 與安全停止；Knowledge Tool 再綁 Session/Domain token；不能開／授權 session、改 policy、發佈知識、清除 estop
  或匯入／刪除素材。CLI 新增 `--agent-scope`；啟動 agent 子程序時移除所有 Runtime token env。
- **控制中心新 IA**：8 一級頁（首頁／小樞／AI 與工作階段／能力與裝置／記憶與知識／
  活動與確認／隱私與安全／設定）＋自動互動保留＋進階 Provider Registry／Knowledge Graph。
  小樞頁 13 個真實素材狀態預覽（未驗證 vs 已驗證對照可見）；AI 頁真實發現／session 卡片
  （claimed-completed 標「回報完成——尚未驗證」）／approval 裁決／Consent Sheet 授權預覽；
  能力頁掃描誠實文案＋未支援能力附具體原因；記憶知識頁含候選複審、素材影響預覽、
  收據、Bundle「本次提供了哪些」；活動頁統一「待我決定」收件匣；首頁「現在」摘要條；
  Global Search＋Command Palette（⌘K，指令只列可執行）。Activity 支援 Agent/裝置/
  Domain 複合篩選，Source Viewer 支援圖片區域、音視訊時間段與程式位置。畫面證據 100 張由 E2E 從
  真 App＋真 daemon 自動擷取（docs/assets/v04-evidence/）。
- API：`/v1/presentation*`、`/v1/proactive-dialogue*`、`/v1/agents*`、
  `/v1/agent-sessions/{id}/{approve,interrupt}`、`/v1/memory*`、`/v1/assets*`、
  `/v1/knowledge*`。CLI：`presentation`／`proactive`／`memory`／`assets`／`knowledge`
  ＋`agents providers|route|approve|interrupt`＋`agents create --workdir --max-cost --allow-write`
  ＋`providers scan`；全域 `--agent-scope` 使用 restricted token。
  Storage schema v3→v7。

### Fixed

- 出貨 persona pack（persona-shu／persona-navigator）因含 `succeeded-verified` 安全鍵
  被驗證器整包拒絕、persona 語句靜默失效（v0.3 對抗審查修正引入的回歸）。
  新增「出貨 pack 必須通過驗證」測試防再發。
- 佇列訊息文字可達 desktop-pet 頻道（is_text_channel 涵蓋）。
- **對抗審查（8 維度 67 agent、59 findings → 38 確認／21 駁回）確認缺陷 38/38 全修**，
  每項附 regression test（該輪 Rust 257→294、vitest 61→81；本次收斂後 336／94）。要點：presentation_ack 改
  persist-成功才發事件（estop 競態不再對事件流謊稱完成）；kill 路徑鎖外化＋pgid
  持久化＋重啟 reap 孤兒＋stdin 有界逾時（卡死 agent 不再擋 estop/close）；GET
  messages 不再對 gateway session 偽造送達；estop/create TOCTOU 與重複 close 修復；
  記憶 far-future horizon 降權漏洞、secret 掃描涵蓋 tags/provenance、過期即拒；
  知識空證據門檻、approve 復活終態、candidate 邊誤降 active、懸空 hash 拒升格、
  LIKE 子字串誤匹配、1000 上限靜默截斷全修；主動對話勿擾真確定性生效；前端
  estop 二段確認＋IME 防誤觸、匯出/拖放/計數/倒數文案誠實化。
- **指定 closing 對抗審查**（41 agents、36 findings → 12 confirmed／24 rejected、
  0 workflow errors）12/12 修復：Governor 原子 reservation、plan single-flight、recipe
  direct-run policy、late receipt evidence、duration overflow、bounded mock state、supersede
  schema、restricted SSE、nested transport error、rolling window、fusion tie-break、driver
  status-chain spoof。

### 已知限制（v0.4 初版快照；已由下方 closing audit 取代）

1. 單一扁平 API token 沿用（v0.3 已知限制①延續；presentation ack 同 token 可偽）。
2. Codex exec fallback 未實作（app-server 不可用時誠實拒絕）。
3. 向量檢索為誠實標示的 lexical-fallback，非語意 embedding（介面可替換）。
4. 影音素材衍生解析（縮圖/OCR/轉錄）未實作；資料模型與片段語法已就緒。
5. OS 層硬體列舉（HID/BLE/MIDI/mDNS/攝影機）誠實未實作，UI 附具體原因。
6. Claude `-p` 模式無互動核可管道；寫入型工作流程為下一階段。
7. 程序化眼球／耳朵疊加層未實作（錨點已輸出；反應鏈烘焙於動畫時間軸）。
8. 生成式主動對話的觸發端排程器為下一階段（閘門＋預算＋metadata 已就緒）。
9. 知識 UI 三末端未接線：update-check 觸發僅 API/CLI、「使用者糾正」專屬入口、
   角色端知識六句固定文案（§17）未接到氣泡（語意在控制中心完整呈現）。
10. agent 子程序孤兒回收 best-effort：pgid OS 快照歸因、重啟 reap 驗證存活＋
    leader＋command；極端 pid 重用可能誤殺、歸因不唯一時誠實放棄（warn）。

### 已知限制（v0.4 closing audit）

1. 同 OS 帳號的 Agent 程序隔離仍依 Codex／Claude Code sandbox；0600 token file
   不冒充完整程序隔離。
2. 本機向量為 sparse subword embedding，不宣稱 neural embedding。
3. OCR／whisper／ffmpeg derivative 依賴本機可選工具；未安裝時明確 unavailable。
4. Windows hardware discovery 尚未真機驗收；driver／權限／sandbox 仍可能讓裝置不可見。
5. 本機 Codex 支援 app-server，因此 exec fallback 以真子程序 fixture 而非真模型驗收。
6. Offline 是 App-level shared state，只保留 desktop／390px 各一張，不複製假頁面。
7. 記憶備份採逐筆驗證還原，不是跨筆 transaction 或 SQLite 覆寫。
8. agent orphan recovery 對無法唯一歸因的 pgid fail-safe 放棄並記錄 warning。

## [0.3.0] - 2026-08-26

### Added — 狀態列常駐、桌面角色、外部裝置、AI Session、感測層

- **狀態列常駐工具**：跨平台 tray／menu-bar（狀態文字永不只靠顏色）、暫停／暫停一小時／
  緊急停止直接走 Rust、「顯示/隱藏桌面角色」、「開啟控制中心」。關閉控制中心視窗改為
  **只隱藏**（首次顯示說明對話框，含 v0.2→v0.3 行為改變告知）；只有「完全結束」才
  優雅關閉內嵌 runtime。登入啟動預設關閉。single-instance 保護。
- **RuntimeSupervisor**：啟動時偵測既有 `interact-ai` daemon → 連線外部（前端切 HTTP
  transport，token 走 IPC）或啟動內嵌 runtime；外部模式下完全結束 app 不會關閉外部
  daemon；health loop 在斷線時誠實降級。狀態：Starting / EmbeddedOwned /
  ConnectedToExternal / Ready / Degraded / Disconnected / Stopping / Stopped。
- **桌面角色小樞**：第二個透明無邊框視窗，原創參數化 SVG rig → 確定性 sprite sheet
  （72 幀 × 3 變體 × 18 動畫）。確定性狀態機：completed=點頭、綠勾只給 verified、
  emergency 凍結一切；click-through（角色 hit-rect 外可穿透）、可拖曳並記憶位置。
- **多模態輸入**：`desktop.companion.interaction`（純語意事件，**永不記錄原始座標**）、
  `desktop.pointer.activity`（本 app 摘要，誠實限制）；點擊快捷操作（確定性）、文字輸入
  ＋資料去向預覽、拖放先預覽再確認。
- **Persona / World / Story packs**：純資料、有界、無可執行內容。**安全語句
  （緊急停止／被阻擋／結果未知／感測使用中）固定不可覆寫**——驗證器標記覆寫企圖，
  resolver 也直接無視。內建 persona-shu／persona-navigator／story-shu-intro。
  表現程度（安靜／自然／活潑）調整氣泡與眨眼節奏。
- **Capability Provider 統一模型**：device/service/application/ai-provider/ai-agent/
  ai-session/companion/human。生命週期禁走捷徑（配對≠安裝≠啟用≠同意），revoked 黏性；
  配對用 sha256 指紋（IP 不是身分）。
- **宣告式 adapter engine**（`interaction-adapter-declarative`）：YAML spec →
  真 HTTP/SSE receptor/actuator，無需寫 Rust。policy-bounded 模板替換（設備只看到限界後
  的值）、`secret://` 參照（env／secret store，不寫進 YAML）、SSRF 防護、
  retry/timeout/idempotency；不支援的 transport（WS/MQTT/Serial/BLE）誠實拒絕。
- **AI Agent Session**：Provider→Agent→Session 分離；有租約（明確續租、lazy 過期）、
  範圍（data/tool/consent）、預算（訊息/成本/時間硬上限）；mailbox 溝通（不互讀對話）；
  `agent.delegate` actuator 走同一 governor 管線；防循環信封（深度/循環/數量/預算）；
  estop 取消所有 session 並阻擋新建；open session 不跨重啟存活；bounded handoff（拒絕
  對話大小的 payload）。
- **感測層**（`adapters-media`）：麥克風 listen 視窗（30 秒硬上限、watchdog 掃描），
  原始音訊只在記憶體、只導出 level 事實，不存不傳、無 STT；預設關＋Intimate＋
  三重確定性閘門（no-estop＋enabled＋session consent）。**無靜默擷取路徑**（status／
  事件／狀態列 glyph／控制中心橫幅／小樞標籤全程反映）。攝影機誠實未實作。
- **新 API**：`/v1/providers*`、`/v1/agent-sessions*`、`/v1/sensors/microphone/listen`、
  `/v1/sensors/stop`；事件新增 `provider.registered/state-changed`、`sensor.started/stopped`。
- **新 CLI**：`interact-ai providers`、`agents`、`sensors`。
- **前端**：一般模式首頁多 Session 視圖、設定頁「桌面角色」區塊；HTTP transport 層
  （同一 typed API 走 Tauri IPC 或外部 daemon，含 fetch-SSE 重播）；Playwright 瀏覽器
  級 E2E（11 測試，真 daemon＋真瀏覽器）。

### Changed
- Storage schema v3：新增 `providers`、`agent_sessions` 表（自動遷移）。
- **誠實性修正**：best-effort 驗證不再把裝置 actuator 的 acknowledged 自動升級成
  completed，除非該 actuator 正式宣告 ack 即代表送達（本機通道）；設備需 observation 才
  completed。內建 canonical tool 現在攜帶正式 human meta（execute/recipe_run 保守標為
  Unknown，因下游影響取決於選中的 actuator）。
- CORS 改為 loopback-only 判定（任何 port：Tauri／vite dev／瀏覽器 E2E）；token 仍是
  實際守門。
- CI 新增瀏覽器 E2E job 與 alsa 系統相依；release 兩個 job 補 `libasound2-dev`。

### Security
- 30-agent 對抗審查（5 維度 find → 逐發現獨立 verify），確認缺陷全數修復（詳見
  `docs/acceptance-evidence.md`）。

## [0.2.0] - 2026-08-26

### Added — 人類理解層（Human Layer）
- **一般／進階雙模式桌面介面**：預設一般模式（首頁／感知來源／回應方式／工具操作／自動互動／同意與安全／活動紀錄／設定），進階模式完整保留原有技術頁面；模式偏好持久化於後端，CLI/API/UI 共用。
- **四層人類語意系統**：manifest 可選 `human` 欄位（presentation／data／effect／consent semantics，缺漏一律保守視為 unknown）→ 內建 44 條目的常見能力中央目錄（zh-TW/en、alias glob）→ 確定性 fallback（技術 ID 分詞＋schema 說明）→ AI 輔助說明（綁定 manifest hash、變更即失效、絕不覆蓋安全事實）。
- **首次設定精靈**：7 步 draft/commit，敏感與對外能力永不預選；套用走同一套 governor 驗證路徑。
- **句子式配方編輯器**：不寫 YAML 建立自動互動；自然語言摘要由結構化 recipe 確定性生成（Rust `summarize`）；YAML↔JSON 經單一模型無損轉換，未知欄位以 serde flatten 完整保留。
- **情境模擬**：安靜時段／缺同意／裝置離線／AI 不可用／低信心／資料過期／已提醒過／緊急停止，重用同一套 pure governor/planner，保證零副作用。
- **主動互動暫停（pause）**：與緊急停止語意分離的一般控制；暫停期間 recipe 不觸發、明確請求照常；持久化、重啟不消失、可設定期限自動恢復。
- **AI 介入決策閘門**：recipe 級 `ai` 策略（never／when-uncertain／generate-text…）；確定性事件絕不呼叫 AI；證據模糊時發布 `ai.assist.requested` 事件，逾時依 `onUnavailable` 確定性處理（fallback／no-action）；外部 AI 可在期限內以 `assists resolve` 回應。
- **緊急停止安全解除流程**：觸發（一鍵、二段確認）與解除（安全頁、顯示原因／恢復清單、高風險不自動恢復）分離。
- **新 API**：`/v1/capabilities/human`、`/v1/catalog`、`/v1/ui/preferences`、`/v1/onboarding/*`、`/v1/pause*`、`/v1/capabilities/{kind}/{id}/ai-description`、`/v1/ai-assists*`、`/v1/recipes/{id}/summary`、`/v1/recipes/{id}/simulate-scenario`、`/v1/recipes/convert`；事件新增 `proactive.paused/resumed`、`ai.assist.requested/resolved`。
- **新 CLI**：`capabilities --human`、`catalog`、`pause`／`resume`、`prefs`、`onboarding`、`describe`、`assists`、`recipes summary`、`recipes simulate --scenario`。
- 前端元件測試（vitest）：誠實性不變量（queued≠completed 等）、能力卡片、權限地圖、對話框、精靈。

### Changed
- Storage schema v2：新增 `ai_descriptions` 表（自動遷移）。
- Recipe JSON Schema 隨模型擴充（`ai`、未知欄位保留）。

## [0.1.3] - 2026-08-26

### Fixed
- 桌面 app：app 層級退出（Cmd+Q／AppleScript quit）現在也會優雅關閉內嵌
  runtime（RunEvent::Exit handler）；先前只有視窗關閉路徑會清理
- 動態註冊的 mock 裝置現在自動配對 `<id>.device-status` 受器，
  `observed` 驗證可完整閉環；`actuators remove` 一併移除配對受器
  （註冊/權限鏈實測發現的缺陷）

## [0.1.2] - 2026-08-26

### Fixed
- 緊急停止 clear 後，閂死（latched）的實體裝置 driver 現在會被重新武裝
  （新增 `Actuator::emergency_clear`，預設 no-op；動作仍不自動恢復）——
  全能力矩陣實測發現的缺陷

## [0.1.1] - 2026-08-26

### Changed
- `self install-skill` 跨 AI 化：自動偵測 Claude Code／Codex CLI／~/.agents／
  Gemini CLI／GitHub Copilot CLI 的 agent home，TTY 下提供選單（預設全選），
  非互動直接全裝；`--dest` 仍可指定任意位置
- 修復 CLI e2e 測試的埠競態（sequential port allocation）

## [0.1.0] - 2026-08-26

### Added
- 首版：12-crate Rust workspace（core／policy／recipe／storage／registry／events／
  runtime／tool-schema／api／cli／adapter-sdk／builtin adapters）
- `interact-ai` CLI（40+ 子指令、`--json` 潔淨輸出、穩定 exit codes、daemon 模式）
- HTTP API（axum、Bearer token、SSE + Last-Event-ID 重播、OpenAPI）
- Deterministic Policy Governor（min() 限界鏈、consent、quiet hours、預算、
  pre-dispatch gate、sticky terminal receipts）
- Recipe 引擎（六種觸發融合、六種編排模式、事件消耗語意、跨重啟狀態持久化）
- Canonical Tool Manifest → OpenAI／Anthropic／Gemini／OpenAPI／JSON-Schema 產生器
  （golden tests）
- 跨 AI Agent Skill（`orchestrate-adaptive-interaction`）
- Tauri 2 桌面控制中心（總覽／受器／動器／工具／配方／政策／時間軸＋緊急停止）
- `interact-ai self` 自我管理（update／uninstall／version／install-skill／install-desktop）
- 文件：ELI5 安裝、特點能力、人類使用手冊、桌面指南（mermaid＋插圖）
- 25-agent 對抗式審查，14 項確認缺陷全數修復；105 測試

[Unreleased]: https://github.com/miles990/adaptive-interaction/compare/v0.5.1...HEAD
[0.5.1]: https://github.com/miles990/adaptive-interaction/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/miles990/adaptive-interaction/compare/v0.4.1...v0.5.0
[0.4.1]: https://github.com/miles990/adaptive-interaction/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/miles990/adaptive-interaction/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/miles990/adaptive-interaction/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/miles990/adaptive-interaction/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/miles990/adaptive-interaction/releases/tag/v0.1.0
