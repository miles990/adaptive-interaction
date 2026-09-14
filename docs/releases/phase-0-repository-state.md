# 階段 0：Repository 真實狀態（2026-09-07）

> 這份文件回答「開工前 repo 到底在哪裡」。所有值都是當場查核，不是從舊報告抄的；
> 再查一次的命令列在每節末尾。階段 0 的入口文件是 [phase-0-progress.md](phase-0-progress.md)。

## 1. 查核時間與環境

| 項目 | 值 |
|---|---|
| 首次查核 | 2026-09-07 11:15 +08:00（Asia/Taipei） |
| 續接再核對（額度中斷後） | 2026-09-07 16:46 +08:00，`git fetch --prune origin` 後 origin/main、tag、open PR 均無變化 |
| OS／核心／CPU | macOS 26.2 (25C56)、Darwin 25.2.0、arm64、Apple M2 Pro |
| Rust／Cargo | rustc 1.94.0 (4a4ef493e 2026-03-02)、cargo 1.94.0（Homebrew；`rust-toolchain.toml` 為 CI 的單一來源） |
| Node／pnpm | v24.5.0／10.27.0 |
| Xcode | 26.6 (17F113)，需 `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer` |
| gh | 2.83.2 |
| Claude Code | 2.1.263，`claude auth status`：已登入（claude.ai，max） |
| Codex CLI | 0.153.4，`codex login status`：已登入（ChatGPT）；`codex app-server --help` 可用 |
| arduino-cli | 1.5.1（Homebrew；ESP32 只做 compile check，無真板） |
| 磁碟 | 開工時 29 GiB 可用、`target/` 不存在（全量重建）；續接時 12 GiB 可用、`target/` 7.4 GB |
| 真 iPhone | iPhone 11 (iPhone12,1)、iOS 26.3.1，有線＋tunnel 連線、Developer Mode 已開；`device-build.sh --check-only` 停在 3/5：Xcode 沒有選 Team（人類操作，未繞過） |

```bash
date '+%Y-%m-%dT%H:%M:%S%z'; sw_vers; uname -m; rustc -V; cargo -V; node -v; pnpm -v; gh --version
claude --version; claude auth status; codex --version; codex login status
```

## 2. Repository、remote、分支與 HEAD

| 項目 | 值 |
|---|---|
| 路徑 | `/Users/user/Workspace/claude-lab/adaptive-interaction` |
| remote | `origin` = `git@github.com:miles990/adaptive-interaction.git`（fetch／push 同） |
| 起始分支／HEAD | `main` @ `78dcda1a3733c97d266ca9b60ad4461c69ca2032`（docs: preserve CI race diagnosis and independent regression evidence） |
| origin/main | `78dcda1a3733c97d266ca9b60ad4461c69ca2032`（與本機一致，兩次查核皆同） |
| 施工分支 | `phase-0/state-recovery-baseline`（自 78dcda1 建立；不把工作樹退回歷史版本） |
| 工作樹 | 開工時乾淨（`git status --porcelain` 空）；只有主 worktree（`git worktree list`） |
| 本機分支 | `wt/drills`（本機獨有、未合併：領先 main 6 commits、落後 55；內容已被 v0.8.0 N5 的 `scripts/drills/` runner 取代，未刪除）；`feature/v0.6.0-foundation`、`feature/v0.6.x-maintainability`、`release/v0.5.0-product-hardening`、`release/v0.5.1-product-hardening` 都已完全併入 main（`git branch --merged main`） |

## 3. 最新 stable tag、tag object、peeled commit

| 項目 | 值 |
|---|---|
| 最新 stable tag | `v0.8.0`（annotated） |
| tag object | `5bca3660afa1d74d7f0844eeb7832e1e25ff9375` |
| peeled commit | `1fa69b8fafed54687119747c652a0869cb991f89` |
| tag 之後的 main | `77e9fc4` docs、`b55c1be` docs、`3ba7364` test（rebind 稽核等待，只改測試）、`78dcda1` docs——**沒有產品程式碼變更**（`git diff --stat 1fa69b8..78dcda1` 只有 docs／tests） |
| 前一版 | `v0.7.0` → `630b4291f6a59444cfb1d8185f757f9dcda9ecc4` |

```bash
git tag --sort=-creatordate | head -3; git rev-parse v0.8.0 v0.8.0^{commit}; git log --oneline 1fa69b8..origin/main
```

## 4. 開放 PR、未合併分支、CI、Release

| 項目 | 值 |
|---|---|
| 開放 PR | 0（`gh pr list --state open`，兩次查核皆空） |
| 遠端未刪分支 | `codex/rebind-audit-test`、`codex/v0.8.0-release-evidence`、`feature/semantic-recovery-convergence`——對應 PR #5／#6／#7 均已 rebase merge；`rebind-audit-test` 相對 main 差異 0 檔，其餘落後 main |
| main CI（78dcda1） | run [34022222409](https://github.com/miles990/adaptive-interaction/actions/runs/34022222409) success |
| 前一次 main CI 失敗 | run 34020702196 @ `b55c1be`：rebind 整合測試競態，`3ba7364`（只改測試）修正；見 [v0.8.0-ci-followup.md](v0.8.0-ci-followup.md) |
| Release | `v0.8.0` published 2026-09-06T07:35:55Z，23 assets，release workflow run 34018696536 success；資產與安裝 smoke 見 [v0.8.0-publication.md](v0.8.0-publication.md) |
| Release candidate CI | 候選 `c15873a` 的 PR #5 四項 CI success 後 rebase 合併，實際 main `1fa69b8` 樹與候選一致（見 [next-convergence-progress.md](next-convergence-progress.md) §5） |

```bash
gh pr list --state open; gh run list --branch main --limit 3; gh release view v0.8.0 --json publishedAt,assets
```

## 5. 五項核對

| 核對 | 結果 |
|---|---|
| 前一輪工作是否已合併 | 是：v0.8.0 收斂（PR #5）、發布證據（#6）、rebind 測試修正（#7）都在 origin/main；`next-convergence-progress.md` §5 第 7 點已寫明「本輪已完成，不重做」 |
| 是否有其他分支已完成本輪想做的能力 | 否：遠端三個分支都已合併且落後 main；`wt/drills` 內容已被 `scripts/drills/` 取代。階段 0 不做新功能，只做盤點／基線／證據 |
| 文件 draft／merged／published 是否符合實際 | 符合：`evidence-index.json` 的 0.8.0 條目 status=published、tag→commit 對得上（`scripts/tests/docs-claims.sh` 會擋）；`CHANGELOG.md` `[Unreleased]` 只有測試維護，並明說「v0.8.0 產品與 tag 不變」 |
| main 的發布後修復是否被誤寫成 tag 已包含 | 否：`3ba7364` 記在 `[Unreleased]` 與 `postReleaseTestCorrections`，`tagUnchanged: true` |
| 當前使用者修改是否與工作範圍重疊 | 否：開工時工作樹乾淨，沒有未提交修改 |

## 6. 本輪產出的 binary 身分（給 E2E 證據引用）

| 產物 | 身分 |
|---|---|
| `target/debug/interact-ai`（第一顆） | 由 HEAD 78dcda1 建置（2026-09-07 11:29），`interact-ai --version` = 0.8.0，sha256 `1e066d84d28b9397c6c6843522ec6a0fbaf692d10ffbea59481afb79860c84c7`；2026-09-07 的基線與 E2E（上一輪 e2e、R1–R8）都用它 |
| `target/debug/interact-ai`（第二顆，**uncertain**） | 上一個 session 記為 2026-09-08 17:29 由同一棵樹重建（`cargo build -p interaction-cli`；產品原始碼未變），sha256 `9b013e955f65ae3c6514487ae76eb0d0e8dee03964044189d81b27ae9d75c871`。**收尾核對（2026-09-14）**：證據目錄 588 檔內沒有任何一檔含這個 hash，且 H-evidence／H-fixture 的內嵌時間戳全部是 2026-09-07T10:20–10:24Z（台北 09-07 18:20–18:24），早於這次重建——所以「H 輪用它」不成立。H 輪所用的 daemon binary **未被記錄**；這顆 binary 的存在只有敘述、沒有歸檔佐證，`target/` 之後已清除，無法再重算 |
| `target/debug/examples/fake_iphone` | 第一顆隨 2026-09-07 建置（sha256 `64ed2d995093064ed5db070705e7830448a11ae991f8d1823d6799429709127e`，R6／R7 使用）；套用 D17 修復（reconnect 不再硬退出）後於 2026-09-07T10:20:35Z 重建，sha256 `188c7dfd0e9ad7b96423625e6d333c6031b6dd30b5ab2feaaf2d4b419f72c5d0`（[`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/fake_iphone.sha256`](evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/fake_iphone.sha256)，H-fixture F-06 使用；模擬 iPhone fixture，非真機）。上一個 session 另記 09-08 17:29 重建出 `02e1d752…`，該 hash 證據內查無來源，同樣 uncertain |
| `apps/interaction-desktop/src-tauri/target/release/bundle/macos/interaction-control-center.app` | 2026-09-06 13:53 建置的 v0.8.0 候選 App，binary sha256 `1103cda7f8491ece2adbaab478056d14a174da47ab8536a2d652c734a0765f19`、Info.plist 版本字串 0.7.0（release-prepare 之前的樹）；與 [2026-09-06 收斂證據](evidence/2026-09-06-convergence/README.md) 的 native runs 同一顆。**不是由 HEAD 重建**：App 的 source `87b0d031` 相對 tag `1fa69b8` 只差四處版本字串（0.7.0→0.8.0）與文件；候選 `c15873a` 與 tag 樹完全相同；HEAD 相對 tag 只有 docs 與一支測試檔（`crates/interaction-runtime/tests/declarative_session_loop.rs`）。原生桌面 E2E 引用它時必須如實標示這個身分 |
