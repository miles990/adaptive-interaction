# 階段 0 原始證據索引

這個目錄是**階段 0**（真實狀態恢復、架構與流程盤點、雙平台真 Agent 驗收、測試基線）的原始執行證據歸檔——
不是敘事文件，是給敘事文件（`phase-0-*.md`）逐條引用的一手 log／JSON／trace。

- **查核 checkpoint**：`78dcda1a3733c97d266ca9b60ad4461c69ca2032`（`main` = `origin/main`，即 `evidenceSourceCheckpoint`）
- **時間**：2026-09-07 11:15 起（額度中斷後於 2026-09-07 16:46、2026-09-08 17:24 再核對；2026-09-08 18:22 網路中斷後續接），Asia/Taipei
- **daemon binary**：`baseline/`／`real-agent-e2e-prior/`／R1–R8 用的是 `1e066d84…`（2026-09-07 11:29 由 HEAD 建置），這是本目錄裡**唯一有歸檔佐證**的一顆（`analysis/commands.json` 的 `binary-sha256`／`binary-version` 兩筆 exit=0）。
- **H 輪（H-evidence／H-fixture／H-native）的日期與 binary 歸屬**：日期已核正為 **2026-09-07 傍晚**——H 輪目錄內嵌的時間戳全部落在 `2026-09-07T10:20–10:24Z`（台北 09-07 18:20–18:24；例：`e2e-runs/H-fixture-attempt1-interrupted/logs/cargo-fmt.txt` START `2026-09-07T10:20:05Z`、`e2e-runs/H-evidence-attempt1-interrupted/term-claude.start.txt` `2026-09-07T10:24:00Z`），**不是 09-08**；`e2e-runs/H-native-attempt1-interrupted/` 自己沒有任何時間戳，執行日期未記錄。所用 `interact-ai` 的 sha256 在證據裡**沒有任何紀錄**（H 目錄沒有 shasum 輸出），因此標 **uncertain**：`phase-0-progress.md` §1 與 `phase-0-repository-state.md` 所稱「`9b013e955f65ae3c6514487ae76eb0d0e8dee03964044189d81b27ae9d75c871`（09-08 17:29 重建）供 H 輪使用」在本證據庫查無來源（`grep -r 9b013e95 .` 只命中本檔），且與上述時間序矛盾，**那兩份敘事文件已在收尾時（2026-09-14）改為 uncertain**：`phase-0-progress.md` §1 與 `phase-0-repository-state.md` §6 現在都寫明 H 輪的 daemon binary 未被記錄、`9b013e95…` 只有敘述沒有佐證；`phase-0-e2e-baseline.md` §1 亦同。
- **H-fixture 的 fixture binary 有紀錄**：`e2e-runs/H-fixture-attempt1-interrupted/logs/fake_iphone.sha256` = `188c7dfd…`（D17 修復後、未提交的版本；同輪 `logs/cargo-build-example.txt` 於 `2026-09-07T10:20:35Z` 重建，`phase-0-e2e-baseline.md` 第 47 行寫的也是這顆）。`phase-0-repository-state.md` 原稱 09-08 重建出 `02e1d752…`，該 hash 在本證據庫同樣查無來源，收尾時已改標 uncertain 並以 `188c7dfd…` 為 H-fixture 所用版本

## 對應的交付文件

- [進度入口](../../phase-0-progress.md)
- [雙平台與多 Session E2E 基線](../../phase-0-e2e-baseline.md)（E0-01～E0-11 正式版；逐案證據路徑直接指到本目錄）——其前身草稿與逐案原始 JSON 留在本目錄的 `analysis/e2e-baseline-draft.md`／`analysis/e2e-baseline.json`，本 README 的目錄地圖表沿用同一批引用
- [已知問題可重現性](../../phase-0-known-issues-reproducibility.md)（L 維度 71 列 + D1–D20）
- [能力恢復矩陣](../../phase-0-capability-recovery-matrix.md)（維度 A–G＋K）
- [測試責任矩陣](../../phase-0-test-responsibility.md)（維度 I）
- [架構依賴與資料流](../../phase-0-architecture.md)（維度 H）
- [文件與現況對照](../../phase-0-docs-vs-reality.md)（維度 J）
- [Repository 真實狀態](../../phase-0-repository-state.md)（binary sha256、worktree、CI 狀態的查核細節）

## 1. 目錄地圖

| 路徑 | 內容 | 對應 E0 案例／維度 | 證據層級 | 是否跑完 |
|---|---|---|---|---|
| `artifact-manifest.json` | 588 筆歸檔清單：original／path／originalSha256／archivedSha256／bytes／normalization | — | — | — |
| `baseline/` | HEAD 78dcda1 的完整測試基線（`scripts/tests/phase0/baseline.sh`）：fmt／clippy／`cargo test --workspace`／Tauri Rust test／pnpm typecheck-test-build／`architecture-checks.sh` 六組／CLI e2e／Playwright／iOS 模擬器 XCTest／ESP32 編譯檢查 | 對應 `phase-0-progress.md` §3 測試基線表 | static／unit／integration／browser／simulator／compile-check（各命令見 `baseline/summary.jsonl`） | 是（20 條命令 exit 全 0，見 `baseline/summary.jsonl` 的 20 行） |
| `baseline/arch-drills-evidence/` | `architecture-checks.sh --drills` 的 N5 演練細節（`drill-*.txt`、`summary.tsv`） | 維度 H | static／fixture | 是（drills=PASS，其餘 group 因未指定旗標 SKIP，見 `summary.tsv`） |
| `baseline/ios-sim-out/` | iOS 模擬器建置與 XCTest 輸出 | — | simulator（非真機） | 是（163/163 passed） |
| `static-matrix/` | 12 維度（A–L）能力盤點：finder→獨立懷疑者 verify→completeness critic | 維度 A–L（A–G＋K 對應能力恢復矩陣、H 對應架構、I 對應測試責任、J 對應文件對照、L 對應已知問題） | static-inspection | 是（每個 dim-*.json 都有 `found`／`verdicts`，dim-L 71 列、dim-A–G/K 各 6–12 列） |
| `analysis/` | 跨案彙整：命令與環境快照、E0 基線草稿與其 JSON、A1–A4 分組草稿、產生腳本 | 見下 | 混合 | 是 |
| `analysis/commands.json` | 8 條旁證命令（binary sha256／版本、head-commit、TaskCancelled grep、provider log grep 等）逐條 stdout/stderr/exit | E0-03／E0-09 佐證 | static-inspection／integration | 是（8/8 exit 0） |
| `analysis/environment-2026-09-07T11-15.json` | 查核當下的完整環境快照（OS／rustc／node／gh／Xcode／Claude Code／Codex 版本、repo 狀態、CI 結果） | — | static-inspection | 是 |
| `analysis/e2e-baseline-draft.md`／`e2e-baseline.json` | E0-01～E0-11 逐案分欄結果草稿（Claude Code／Codex／fixture／native-desktop／real-iPhone） | E0-01…E0-11 | 混合（草稿本身標明每案證據層級） | 是（草稿本身完整，但檔名／progress.md 引用的正式檔名尚未產生，見上方「對應的交付文件」） |
| `analysis/A1-E0-02.json` | E0-02（派工／多輪／續接）分組草稿輸入 | E0-02 | real-agent／integration | 是 |
| `analysis/A2-E0-03.json` | E0-03（取消／中斷／緊急停止）分組草稿輸入 | E0-03 | real-agent | 是 |
| `analysis/A3-E0-06-07.json` | E0-06／E0-07（同型與跨型多 session 併發）分組草稿輸入 | E0-06、E0-07 | real-agent | 是 |
| `analysis/A4-E0-04.json` | E0-04（核心崩潰與重啟恢復）分組草稿輸入 | E0-04 | real-agent | 是 |
| `analysis/build_a2.py`／`build_final.py` | 產生上述草稿的彙整腳本（把 `commands.json`＋各分組 JSON＋`result-R*.json` 併成 `e2e-baseline-draft.md`） | — | — | 是（腳本本身，非執行紀錄） |
| `harness-used/` | 上一輪（`real-agent-e2e-prior/`）用的三支 Python harness 快照：`agent_smoke.py`／`multi_session.py`／`restart_test.py` | — | — | 是（快照，非本身的執行結果） |
| `real-agent-e2e-prior/claude-smoke-1` | 真 Claude Code 單輪派工 smoke | E0-02 | real-agent | 是（`final.state=claimed-completed`，wallSeconds 11.2） |
| `real-agent-e2e-prior/codex-smoke-1` | 真 Codex 單輪派工（未核可） | E0-02 | real-agent | 是（`final.state=waiting-for-consent`，wallSeconds 303.34，正確阻擋非失敗） |
| `real-agent-e2e-prior/codex-smoke-2` | 真 Codex 單輪派工（核可後） | E0-02 | real-agent | 是（`final.state=claimed-completed`，wallSeconds 17.01） |
| `real-agent-e2e-prior/claude-cancel-1` | 真 Claude Code 取消 | E0-03 | real-agent | 是（`final.state=failed`，見 D1 缺陷） |
| `real-agent-e2e-prior/codex-cancel-1` | 真 Codex 取消 | E0-03 | real-agent | 是（`final.state=cancelled`） |
| `real-agent-e2e-prior/multi-C-claude-x2` | 同型（Claude×2）併發 | E0-06 | real-agent | 是 |
| `real-agent-e2e-prior/multi-D-codex-x2` | 同型（Codex×2）併發 | E0-06 | real-agent | 是 |
| `real-agent-e2e-prior/multi-E-claude-codex` | 跨型（Claude×Codex）併發 | E0-07 | real-agent | 是 |
| `real-agent-e2e-prior/restart-claude-kill` | SIGKILL 重啟恢復（Claude session） | E0-04 | real-agent | 是（`signal=KILL`，wallSeconds 28.23） |
| `real-agent-e2e-prior/restart-codex-kill` | SIGKILL 重啟恢復（Codex session） | E0-04 | real-agent | 是（`signal=KILL`，wallSeconds 6.31） |
| `real-agent-e2e-prior/discovery-*.json`／`discovery-daemon.txt` | agent 探索／路由端點的旁證快照 | 維度 A／E0-02 前置 | integration | 是 |
| `e2e-runs/R1` | E0-03 取消／中斷（真 Claude＋真 Codex）與 A-07 緊急停止 | E0-03、A-07、B-07 | real-agent | 是（見 `result-R1.json`；8 案：1 個 `product-failed`（Claude interrupt→failed，D1）、2 個 `not-implemented`（B-07 stop-vs-cancel）、其餘 `completed`） |
| `e2e-runs/R2` | E0-05 記憶→Context Bundle→修改／刪除→新任務（真 Claude） | E0-05 | real-agent／integration | 是（見 `result-R2.json`；7 案全 `completed`） |
| `e2e-runs/R3` | E0-09 Agent 提問→人類延遲回答→接續 | E0-09 | real-agent／integration／static-inspection | 是（見 `result-R3.json`；4 案：自動偵測 `not-implemented`、其餘 3 案 `completed`） |
| `e2e-runs/R4` | E0-10 權限：拒絕／唯讀／授權寫入／驗證／撤銷／過期／續開不放寬（真 Claude＋真 Codex） | E0-10 | real-agent／integration | 是（見 `result-R4.json`；10 案：4 個 `correctly-blocked`（`E0-10-readonly-write-claude`／`-codex`、`E0-10-resume-not-wider`、`E0-10-agent-token-forbidden`）、6 個 `completed`） |
| `e2e-runs/R5` | E0-11 備份匯出／還原／重啟保留 + E0-01 API 可驗部分（首次設定／偏好持久化／重啟） | E0-01、E0-11 | integration | 是（見 `result-R5.json`；6 案：1 個 `correctly-blocked`（壞資料）、其餘 `completed`） |
| `e2e-runs/R6` | E0-08 iPhone fixture（配對／雙向／停止未知／AIP／撤銷／核心離線） | E0-08 | fixture／not-run | 是（見 `result-R6.json`；7 個 fixture 案全 `completed`，`E0-08-real-iphone` 為 `needs-environment`） |
| `e2e-runs/R7` | native-desktop E2E：首次設定恢復、設定匯出匯入、工作取消、mobile fixture 走查（真 Tauri App，2026-09-06 舊 bundle） | E0-01、E0-03、E0-08、E0-11（native 分欄） | native-desktop | 是（見 `result-R7.json`；4 案：1 個 `completed`（preset-recovery）、3 個 `harness-failed`（settings／work-cancel／mobile，AX helper 缺陷 D18／D19） |
| `e2e-runs/R8` | K-04 原生續接（`resumeProviderSessionId`）與授權保持（真 Claude＋真 Codex） | K-04 | real-agent／integration | 是（見 `result-R8.json`；6 案：4 個 `completed`、2 個 `correctly-blocked`） |
| `e2e-runs/H-evidence-attempt1-interrupted` | 2026-09-07 傍晚（UTC 10:24，見 `term-*.start.txt`）補證據：真 Claude／真 Codex 的 SIGTERM 乾淨關閉重啟恢復（`analysis/e2e-baseline-draft.md` 未驗清單第 5 項） | E0-04（SIGTERM 路徑） | real-agent | **是**：`term-claude`／`term-codex` 兩支子跑都以 `EXIT=0`＋完整 `RESULT {...}` 收尾（`state-after-restart=expired`，孤兒回收訊息各自正確），見下方第 7 節 |
| `e2e-runs/H-fixture-attempt1-interrupted` | 2026-09-07 傍晚（UTC 10:20–10:23，見 `logs/cargo-fmt.txt`／`logs/run.txt`）補證據：D17 `fake_iphone` 硬退出修復後的 F-06 四階段重跑 | E0-08（F-06 核心離線） | fixture | **是**：`logs/run.txt` 走完 PHASE A–E，`=== 完成 ===` 收尾，3 個有條件 PASS 斷言（B1／C1／D1）全部 PASS、B2／D2 是資訊性檢查（`run-f06.sh` 不分岔 FAIL，附帶 `alive` 值皆為 `alive`），`cargo-{fmt,clippy}` 與 `cargo-build-example` exit 0、`cargo-test-mobile-loop` 81 passed／0 failed |
| `e2e-runs/H-native-attempt1-interrupted` | H 輪同批（目錄內無時間戳，執行日期未記錄）對 D19（AX dump 隨 session 變慢逾時）的定位嘗試 | E0-08 native（診斷用，非正式案例） | native-desktop（probe，非產品測試） | **否**：`probe.py` 自陳「Probe (not a product test)」；`trace.json` 第 8 步 `batchdump` 已 `exit 1`，`dump-fast.txt` 為空；腳本原本會接著寫 `panel-1-initial.txt`／`sheet-*.txt`／`panel-2-after-goto.txt`／`main-final.txt`，但輸出目錄裡完全沒有這些檔——中途被中斷，未得出結論（與 `phase-0-known-issues-reproducibility.md` D19 段落「該輪被中斷，未得出結論」一致） |

R1–R8 每案的詳細前置條件／步驟／預期／實際證據路徑見對應 `result-R*.json` 的 `cases[]`；跨案總表見 `analysis/e2e-baseline-draft.md`。

## 2. 路徑別名表

文件與 JSON 草稿裡引用證據時用的是彙整當下的相對寫法，和本目錄實際落地的路徑不同：

| 文件／JSON 裡的寫法 | 本目錄實際路徑 |
|---|---|
| `runs/Rn/…` | `e2e-runs/Rn/…` |
| `e2e/<name>/…`（如 `e2e/claude-smoke-1/result.json`） | `real-agent-e2e-prior/<name>/…` |
| `matrix/…` | `static-matrix/…` |

副檔名改名規則（`scripts/tests/phase0/archive-evidence.py` 的固定規則）：

| 來源副檔名 | 歸檔副檔名 |
|---|---|
| `.log` | `.txt` |
| `.stdout` | `.stdout.txt` |
| `.stderr` | `.stderr.txt` |

例：草稿裡的 `runs/R5/E0-01-step1.log` = 本目錄的 `e2e-runs/R5/E0-01-step1.txt`；`e2e/claude-cancel-1/daemon.log` = `real-agent-e2e-prior/claude-cancel-1/daemon.txt`。

## 3. 沒有歸檔的東西

- **`home/`、`*.db`／`*.db-wal`／`*.db-shm`、`api-token`／`api-agent-token`**：`archive-evidence.py` 一律略過名稱含 `home` 的目錄與這幾種檔名（憑證與 SQLite 永遠不進 repo）。實查本目錄：`find . -iname '*home*' -type d`、`find . -iname '*.db*'`、`find . -iname '*api*token*'` 三個都是空結果。
- **workdir 內容**：已全數歸檔（34 個檔，全部是測試自建的 `NOTES.md`／少數 `hello.txt`），沒有被排除的部分——agent 真正的回覆內容不落在 workdir，而是在下一項的 provider 本機紀錄裡。
- **provider 端本機紀錄**：`~/.claude/projects/…`、`~/.codex/sessions/…` 是 Claude Code／Codex 自己的本機 log，本輪只唯讀取用來核對事實（例如 `analysis/commands.json` 的 `claude-provider-interrupt-lines`／`codex-turn-aborted`），**未歸檔**進本目錄。
- **超過 2 MB 只留 hash 的檔**：`archive-evidence.py --max-bytes` 預設 2,000,000 bytes，超過的檔案只記 `originalSha256`、`archivedSha256` 設 `null`、`normalization` 寫 `"not copied (>2000000 bytes); hash only"`。**實查 `artifact-manifest.json` 588 筆，這種項目 0 筆**——本輪最大的歸檔檔案是 `baseline/arch-drills-evidence/drill-current-cli.txt`（303,219 bytes），遠低於門檻。
- **原生 App bundle 與 daemon binary**：只留 sha256，本身不歸檔進本目錄——daemon binary 的 sha256 在 `analysis/commands.json`（`binary-sha256`／`binary-version`）與 `analysis/e2e-baseline-draft.md` 開頭；原生 App bundle（`apps/interaction-desktop` release build，2026-09-06 的 0.7.0 bundle）的 sha256 只寫在 `analysis/e2e-baseline-draft.md` 開頭段落。

## 4. 怎麼驗證

逐檔重算 `archivedSha256` 並與 manifest 比對（可直接貼上執行）：

```bash
cd docs/releases/evidence/2026-09-07-phase-0 && python3 -c "
import json, hashlib
d = json.load(open('artifact-manifest.json'))
files = d['files']
ok = 0
mismatch = []
for f in files:
    h = hashlib.sha256(open(f['path'],'rb').read()).hexdigest()
    if h == f['archivedSha256']:
        ok += 1
    else:
        mismatch.append((f['path'], f['archivedSha256'], h))
print(f'{ok}/{len(files)} 相符')
for m in mismatch:
    print('MISMATCH', m)
"
```

**本檔撰寫時已執行一次，結果：588/588 相符，0 筆不符。**

統計摘要（同一份 manifest 算出來）：

- 總筆數：588
- 各頂層目錄筆數：`analysis` 11、`baseline` 70、`e2e-runs` 441（其中 `H-evidence-attempt1-interrupted` 12、`H-fixture-attempt1-interrupted` 39、`H-native-attempt1-interrupted` 8、`R1` 48、`R2` 39、`R3` 26、`R4` 56、`R5` 23、`R6` 30、`R7` 121、`R8` 39）、`harness-used` 3、`real-agent-e2e-prior` 48、`static-matrix` 15
- byte-identical（非文字檔或未受正規化影響）：235 筆；經文字正規化（去除行尾空白與多餘結尾空行）：353 筆
- 超過 `--max-bytes` 只留 hash（未複製內容）：0 筆

## 5. 怎麼產生

用 `scripts/tests/phase0/archive-evidence.py`：

```bash
python3 scripts/tests/phase0/archive-evidence.py \
  --checkpoint <sha> \
  --dest docs/releases/evidence/2026-09-07-phase-0 \
  --note "…" \
  SRC_DIR:DEST_SUBDIR [SRC_DIR:DEST_SUBDIR ...] \
  [--exclude-dir home --exclude-dir workdir]
```

規則摘要：

- `.log`→`.txt`、`.stdout`/`.stderr`→`.stdout.txt`/`.stderr.txt`；文字檔（`.log/.stdout/.stderr/.txt/.md/.tsv/.jsonl 除外/.yaml/.yml/.py/.sh/.applescript`）正規化行尾空白與結尾多餘空行；JSON／`.jsonl` 原樣保存。
- 一律略過名稱含 `home` 的目錄與其他 `--exclude-dir`；FIFO／socket／symlink 不歸檔；檔名為 `api-token`／`api-agent-token`／`*.db`／`*.db-wal`／`*.db-shm` 的檔永遠不進 repo。
- 超過 `--max-bytes`（預設 2,000,000）的檔只記 hash、不複製內容。
- 憑證掃描：每個來源目錄若有 `home/state/api-token`／`api-agent-token`，其值不得出現在任何被歸檔的檔案裡；另外掃 `Bearer <token>`、`sk-`、`ghp_` 形狀的字串（`.py`/`.sh` 除外）。任何一項命中即整次執行中止（只清掉這次寫入的檔，不動 `--dest` 內既有內容），不做自動改寫。
- 產出 `artifact-manifest.json`：`{evidenceSourceCheckpoint, note, files:[{original, path, originalSha256, archivedSha256, bytes, normalization}]}`。

## 6. `H-*-attempt1-interrupted` 命名說明

三個目錄名稱都帶 `attempt1-interrupted`，因為**當時外層的 workflow session 在跑完 H 輪之後、彙整階段被中斷**（額度／網路中斷，見 `phase-0-progress.md` §1 的時間軸），目錄名稱是那次中斷 session 遺留下來的字面命名，**不代表目錄內每個執行本身都沒跑完**。**執行時間依內嵌時間戳是 2026-09-07T10:20–10:24Z（台北 18:20–18:24），不是 09-08**（H-native 無時間戳，日期未記錄）；所用 `interact-ai` binary 的 sha256 未被記錄，見開頭「H 輪的日期與 binary 歸屬」那一點。逐一核對 `run.txt`／`*.stdout.txt`／`trace.json` 的 EXIT／RESULT／PASS 行後：

- **`H-evidence`：已完成（執行於 2026-09-07T10:24:00–10:24:28Z／10:24:45–10:24:51Z，見 `term-claude.start.txt`／`term-codex.start.txt`；所用 daemon binary sha256 未記錄）。** `term-claude.stdout.txt`／`term-codex.stdout.txt` 都以 `EXIT=0` 收尾，且各自的最後一行是完整的 `RESULT {"agent": …, "signal": "TERM", "stateAfterRestart": "expired", …}`；`result.json` 裡的 `statusAfter`／`after` 欄位齊全，補上了 `analysis/e2e-baseline-draft.md`「沒有驗到」清單第 5 項（`restart_test.py --signal TERM`）原本缺的執行期證據。
- **`H-fixture`：已完成（執行於 2026-09-07T10:20:05–10:23:42Z，見 `logs/cargo-fmt.txt` START 與 `logs/run.txt`；所用 daemon binary sha256 未記錄，fixture 為 `188c7dfd…`）。** `logs/run.txt` 完整跑完 PHASE A（配對連線）／B（SIGKILL 後 reconnect-failed 且 process 存活）／C（同 home 重啟後恢復）／D（撤銷後 auth-fail）／E（收工），以 `=== 完成 ===` 結尾；斷言強度分兩級：**3 個有條件 PASS 斷言**（B1 `reconnect-failed`、C1 第 2 次 `connected`、D1 `auth-fail`，`run-f06.sh` 第 104/106、120、135 行有 if／else 分支）全部落在 PASS，**2 個資訊性檢查**（B2 第 109 行無條件印出 `PASS/FAIL B2 (process 存活): …`、D2 第 137 行連 PASS/FAIL 字樣都沒有，都只印 `alive` 值靠人工判讀，腳本本身不會 FAIL）——`logs/run.txt` 第 15／29 行兩者的 `alive` 值都是 `alive`，但這兩項不等於斷言；`logs/cargo-{fmt,clippy}.txt`、`cargo-build-example.txt` exit 0，`cargo-test-mobile-loop.txt` 顯示 `81 passed; 0 failed`。這是 `phase-0-known-issues-reproducibility.md` D17（`fake_iphone` 硬退出）修復後的驗證證據。
- **`H-native`：確實被中斷，未完成。** `probe/probe.py` 檔頭自陳「Probe (not a product test)」，是對 D19（AX dump 隨 session 累積變慢、逾時）的定位嘗試而非正式 E0 案例；`probe/p1/trace.json` 第 8 步 `batchdump` 已 `exit 1`、`dump-fast.txt` 寫出空字串，腳本理應接著产生的 `panel-1-initial.txt`／`sheet-1-empty.txt`／`sheet-2-filled.txt`／`panel-2-after-goto.txt`／`windows-after-open.txt`／`main-final.txt` 在輸出目錄裡完全不存在，代表流程在寫完 `dump-fast.txt`／`dump-slow.txt` 之後、進入 Open 面板匯入驗證之前就被中止。`phase-0-known-issues-reproducibility.md` D19 段落的結論與此一致：「該輪被中斷，未得出結論」，D19 本身仍是「已修、未驗證」狀態。
