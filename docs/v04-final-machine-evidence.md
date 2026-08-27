# v0.4 Closing Machine Evidence

日期：2026-08-28（Asia/Taipei）。基準：本機 `main` @
`0aa8733ff8f5d7632d59a955a16c08cf1458a92e`；同步時 `origin/main` @
`75f913d9946c3221b7f47136755b2af08739713d`。本機 main 原有 12 個未推送 commit，
本輪工程仍是 working-tree diff；依 repo 規則沒有自行 commit、push、release、deploy 或開 PR。

## 完成判定

- `docs/capability-completion-matrix.md`：**25/25 complete、0 partial、0 missing**。
- tracked diff：115 files、8,504 insertions、678 deletions；另有新 Rust/TS 模組、tests、
  machine-evidence document 與狀態 PNG。完整逐檔清單：`git status --short`。
- Storage 自動 migration：schema v3→v7，依序新增 Provider/Agent Session、10 層記憶、
  CAS/Knowledge Graph/FTS5、Knowledge Receipt、asset derivatives；舊資料逐版遷移，
  `PRAGMA user_version` 保證冪等。Schema/tool golden files已重生。

## 測試命令與實際結果

| 命令 | 實際結果 |
|---|---|
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0，0 warnings |
| `cargo test --workspace` | **336 passed，0 failed，0 ignored**；doc-tests 0 項不計 |
| `cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml` | **4 passed，0 failed** |
| `cd apps/interaction-desktop && pnpm typecheck` | exit 0 |
| `cd apps/interaction-desktop && pnpm test` | **11 files，94 passed，0 failed** |
| `cd apps/interaction-desktop && pnpm build` | exit 0，1863 modules；2 則 Vite chunking warning，無錯誤 |
| `cd apps/interaction-desktop && pnpm test:e2e` | **23 passed，0 failed**；真 daemon＋Chromium，49.9s |
| `./scripts/v03-cli-e2e.sh` | **51 passed，0 failed**；真 daemon＋restricted token＋fixture process |
| `pnpm tauri build --debug --bundles app` | debug `.app` bundle 成功，用於 native Tray 視覺驗收 |
| `git diff --check` | exit 0 |

Rust 336 是所有 test binary 的 `passed` 數加總；沒有用「cargo exit 0」替代實際數量。

## 指定對抗審查

`.claude/workflows/adversarial-review-adaptive-interaction.js` 最終成功 run：

- workflow `wf_856f48f8-3e9`
- session `911ad58c-d833-4e34-942d-390126c27d95`
- 41 agents、0 workflow errors、617 秒、約 1.73M subagent tokens
- 36 findings → 12 confirmed／24 rejected
- 12/12 confirmed 已修復並有 regression test

確認項：Governor 使用量/費用的原子 reservation、同 plan single-flight、recipe direct-run
政策繞過、watchdog 後 late driver evidence、超大 duration panic、mock 集合無界、
supersede schema 缺 evidence/domain、restricted SSE 洩漏、前端 nested error、rolling
hourly window、equal-quality fusion、driver completion-chain spoof。舊嘗試的「存活約
85 分鐘」只是 OS elapsed time，不能證明完成；本文件只採 final output/journal。

## 真實 Codex／Claude Code Connector

- Discovery：Codex `0.150.1`、Claude Code `2.1.247`；兩者 `loggedIn:true`。Runtime
  沒有讀取、複製或保存 credential。
- Codex app-server：Session `asession-cd2be00c-2a36-4b9d-af41-b3a6af703d3b`，Provider
  Thread `01a042f2-3de0-7cd0-897f-a478e41b9687`。讀 README 的 command approval 先到
  `waiting-for-consent`，人類只核可 request `0` 後才執行；Mailbox 收到
  `CONNECTOR_OK`。狀態只到 `claimed-completed`，另以 `rg` 獨立比對 README。
- Claude stream-json：Session `asession-8508e873-23a8-46a7-99e0-0ebc253d0f5a`，
  Provider Session ID、stream events、cost、Mailbox 均落 Runtime；同樣只到
  `claimed-completed`。兩個 Session 均已關閉。
- 本機新版優先 app-server，不能強迫 Codex 走 fallback；`exec --json`／resume／
  malformed event／cancel／process-tree 由真子程序 fixture 驗證，不冒充真模型額度。

## 控制中心與畫面證據

`docs/assets/v04-evidence/` 有 **100 PNG**：

- 9 個一級頁：首頁、小樞、AI 與工作階段、能力與裝置、記憶與知識、自動互動、
  活動與確認、隱私與安全、設定。前八個需求頁之外，Automation 是 v0.3 相容頁。
- 每頁 desktop 1200×800 與 narrow 390×844 的 normal/empty、loading、
  error/unknown、waiting、emergency。
- offline 是 App-level shared Runtime state，desktop／390px 各一張；Runtime 離線時
  不造九份仍可導覽的假頁。
- 另有 Global Search、hardware scan、native Control Center、native Companion、native Tray。

狀態來源：normal/empty 使用全新隔離 home；loading 延遲真 transport；error/unknown
中斷一個真 request；waiting 建立真 Knowledge Candidate；emergency 呼叫真 estop；
offline 連到不存在的 Runtime。沒有 hard-code success payload。

`live-tray-menu.png` 已在打包後真實 Tauri App 中展開 menu bar item 再擷取，可讀到：
Runtime 狀態、主動互動、AI Session 數、開啟控制中心、顯示/隱藏角色、暫停、
緊急停止、設定與完全結束。舊的誤標控制中心圖片已被替換。

## Artifact hashes

```text
94b07c1634b985ba536b5abe9ce5f90631a38ea89a7884a8e83b1df7557120b5  target/debug/interact-ai
83faf1fb78ab57e0c1051583be1dc57943c5e1ea58f6762abc6e242b7fa5f81d  dist/assets/index-E7AmWIid.css
c6c67559c1473bb2903a2140554857b12f4df62013e8c9f4e93793400bd00a56  dist/assets/index-af9L51q1.js
5f4888c732abfa00504dd279667f20069a1504ae32dcf3e31479b44c0f9e9cfa  dist/assets/webview-aQJfPrwr.js
7308eec33457e0da3ffb1cf9ceb6d28337404e73c2c3b4cd272920ed959fd02e  dist/assets/window-QpEaAqrM.js
62c86aa8b5ffc25329150bc461c2258515f667359381686378e936d9066a62c6  skills/orchestrate-adaptive-interaction/SKILL.md
25331b32f76247b9e28989b62740b140beeaf0ce2d8027a7a3c49bcacb8305bf  live-tray-menu.png
73b783db21b9936227d07b914500a46f3dcc43fa46a882a84dccc2621e7eccc9  ordered SHA-256 manifest of 100 PNGs
```

重算最後一項：

```bash
find docs/assets/v04-evidence -maxdepth 1 -type f -name '*.png' -print0 \
  | sort -z | xargs -0 shasum -a 256 | shasum -a 256
```

Skill 已重新安裝到 Claude Code、Codex、`~/.agents`、Gemini、Copilot 五處；五份
`SKILL.md` SHA-256 均等於 repo 的 `62c86aa8…a62c6`。

## 實作分組／修改檔案

- Domain/schema：`interaction-core` discovery/domain-pack/agent/knowledge、recipe model/fusion、
  canonical tool schemas與五份 golden export。
- Runtime/storage：hardware、presentation、proactive、gateway、agents、memory、knowledge、
  curator、activity、executor、SQLite v7 migration。
- Adapters/connectors：builtin bounds、adapter SDK receipt merge、Codex app-server/exec、
  Claude stream-json、Unix process group。
- API/CLI/Skill：routes/auth/SSE/inbox、CLI commands/e2e、跨 AI Skill references。
- Desktop：Tauri supervisor/tray、九頁 IA、Global Search、Behavior renderer、Context/Source/
  Receipt viewers、backup restore、responsive CSS與 tests。
- Docs/evidence：README、Architecture、Features、Install、User/Desktop guides、CHANGELOG、
  acceptance、gap analysis、Capability Matrix、100 PNG。

## 保留風險與重跑條件

- Windows 需在真 Windows 主機重跑 workspace／desktop tests與 hardware scan；目前只承諾
  編譯介面與誠實 unsupported，不宣稱本機驗收。
- OCR/whisper/ffmpeg 缺少時是 `unavailable`；安裝對應本機工具後，以 `assets derive`
  重跑即可取得該類 derivative。
- 麥克風真裝置擷取仍需要人在場授予 OS 權限與 Consent；自動測試只使用確定性 source，
  且已證明掃描不啟動感測。
- 本機兩個 Agent 的登入／版本會改變；重跑前使用 `claude auth status`、
  `codex login status` 與 `interact-ai agents providers --json` 檢查。
