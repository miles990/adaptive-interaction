# 階段 0：專業測試責任矩陣與測試精簡處置

> **這份文件是什麼**：階段 0「真實狀態恢復」的測試面交付。它回答兩個問題——(1) 這個產品的每個重要行為
> **應該由哪一層測試負責**、現在**實際被誰保護**、**缺口在哪**、**什麼時候會被執行**；(2) 哪些既有測試是
> 低價值的，該怎麼處置。**本階段只記錄處置，不刪除、不改寫任何一支測試**；下面的「處置」欄是提案，不是已完成的事實。
>
> **來源**：(a) 靜態盤點與懷疑者複核 `docs/releases/evidence/2026-09-07-phase-0/static-matrix/dim-I.md`／`dim-I.json`
> （18 條 claim，其中 3 條被獨立懷疑者 refuted，見附錄 A）；(b) 本輪基線實跑的原始 log
> `docs/releases/evidence/2026-09-07-phase-0/baseline/`（HEAD `78dcda1`，2026-09-07）；(c) 真 Agent E2E 結果
> `docs/releases/evidence/2026-09-07-phase-0/analysis/e2e-baseline.json`；(d) 本文件作者對每個引用的
> file:line 與計數所做的**獨立重新查核**（方法見 §0.3）。
>
> **怎麼再產生**：見 §0.3。所有計數命令都可在 `78dcda1` 上重跑並得到同一個數字。

階段 0 的其他交付：[Repository 真實狀態](phase-0-repository-state.md)、[分階段計畫](phase-0-roadmap.md)、
以及同批交付的 `phase-0-capability-recovery-matrix.md`、`phase-0-e2e-baseline.md`、`phase-0-known-issues-reproducibility.md`。

---

## 0. 用語、界線與再產生

### 0.1 狀態分類（只用這六個）

本文件把分類用在「**這個行為的測試保護**」上，不是用在產品功能上：

| 值 | 在本文件的意思 |
|---|---|
| `implemented-and-connected` | 有針對這個不變量寫的測試，**而且**接在會自動執行的觸發上（CI job 或發布關卡） |
| `implemented-partial` | 有測試但只覆蓋部分分支／部分端；或有測試但只在人工觸發的 runner 裡 |
| `defined-only` | 只有型別／enum／常數／文件宣告，沒有走過正式路徑的測試 |
| `absent` | 全 repo 查不到針對這個不變量的測試 |
| `confirmed-defect` | 本輪實跑證明測試機制本身有缺陷（例如靜默變綠） |
| `needs-investigation` | 靜態查核找不到，但無法排除藏在別處；需要人再查一次 |

### 0.2 證據層級（只用這些）

`static-inspection`／`unit`／`contract`／`fixture`／`simulator`／`integration`／`browser`／
`native-desktop`／`real-agent`／`real-iphone`／`real-hardware`／`not-run`。

本文件的實跑數字全部來自 `docs/releases/evidence/2026-09-07-phase-0/baseline/`；**所有「缺口」判斷都是
`static-inspection`**（在 `78dcda1` 上 grep／讀原始碼得到），不是實跑證明。
**真 iPhone 與真硬體（ESP32 真板）在本階段的測試證據為 0 筆**——`ios-simulator` 是模擬器，
`firmware-*` 只是 arduino-cli 編譯檢查，`esp32_sim_conformance` 是 python3 模擬器。

誠實階梯同樣適用於測試本身：**測試存在 ≠ 測試會被執行；測試會被執行 ≠ 它會在該紅的時候紅。**

### 0.3 再產生（HEAD `78dcda1`）

```bash
git rev-parse HEAD                       # 78dcda1a3733c97d266ca9b60ad4461c69ca2032

# 一、資產盤點（本文件所有 Rust 計數都用這條規則，與 scripts/tests/docs-claims.sh 同一條）
python3 - <<'PY'
import re, glob
rule = re.compile(r"#\[tokio::test|#\[test\]")
def c(ps): return sum(len(rule.findall(open(p, encoding='utf-8', errors='replace').read())) for p in ps)
src = [p for d in glob.glob('crates/*/src') + glob.glob('adapters/*/src')
       for p in glob.glob(d + '/**/*.rs', recursive=True)]
print('src', len(src), c(src))
for d in sorted(glob.glob('crates/*/tests')) + ['tests/e2e/tests']:
    fs = sorted(glob.glob(d + '/*.rs'))
    if fs: print(d, len(fs), c(fs))
PY

# 二、前端／E2E／iOS 宣告數
cd apps/interaction-desktop && find src -name '*.test.ts' -o -name '*.test.tsx' | wc -l
grep -rhE '^\s*(it|test)(\.each\([^)]*\))?\(' src --include='*.test.ts' --include='*.test.tsx' | wc -l
ls e2e/*.spec.ts | wc -l && grep -rhE '^test\(' e2e/*.spec.ts | wc -l
cd - && grep -rhE '^\s*func test' apps/interaction-ios --include='*.swift' | wc -l

# 三、基線實跑（本文件引用的 passed／failed 全部來自這一輪的 stdout）
PHASE0_OUT=<dir> scripts/tests/phase0/baseline.sh      # 20 步；證據已歸檔到 evidence/.../baseline/

# 四、Playwright 每支 spec 的實際牆鐘時間（§3.4 的 I-18 用它改判）
python3 - <<'PY'
import re
from collections import defaultdict
txt = open('docs/releases/evidence/2026-09-07-phase-0/baseline/pw-e2e.stdout.txt',
           encoding='utf-8', errors='replace').read()
tot, cnt = defaultdict(float), defaultdict(int)
for m in re.finditer(r'›\s*(e2e/[\w\-.]+\.spec\.ts):\d+:\d+ ›.*?\((\d+(?:\.\d+)?)(m?s)\)', txt):
    f, v, u = m.group(1), float(m.group(2)), m.group(3)
    tot[f] += v / 1000 if u == 'ms' else v; cnt[f] += 1
for f in sorted(tot, key=lambda x: -tot[x]): print(f'{tot[f]:7.1f}s {cnt[f]:3d} {f}')
PY
```

---

## 1. 測試資產盤點

> 「宣告數」＝原始碼裡的測試屬性／`it(`／`test(`／`func test` 個數；「本輪實跑」＝
> `docs/releases/evidence/2026-09-07-phase-0/baseline/` 各步 stdout 的數字。兩者**不一定相等**（§1.7）。

### 1.1 Rust（workspace，`cargo test --workspace`）

| 位置 | 檔數 | 宣告測試 | 備註 |
|---|---:|---:|---|
| `crates/*/src` ＋ `adapters/*/src`（單元） | 125 | 404 | 純函式與模組內測試 |
| `crates/interaction-runtime/tests` | 23 | 449 | 最大宗；`declarative_session_loop.rs` 單檔 2,696 行 |
| `crates/interaction-adapter-declarative/tests` | 8 | 124 | 含 `esp32_sim_conformance.rs` 24 支、`protocol_honesty.rs` 36 支 |
| `crates/interaction-session/tests` | 10 | 121 | `session.rs` 49、`session_hardening.rs` 17 |
| `crates/interaction-character/tests` | 6 | 85 | `gateway.rs` 37、`negotiation.rs` 16、`migration_registry.rs` 10 |
| `crates/interaction-api/tests` | 3 | 39 | `api_e2e.rs` 33、`consent_e2e.rs` 4、`providers_e2e.rs` 2 |
| `crates/interaction-cli/tests` | 2 | 18 | `cli_e2e.rs` 8、`release_provenance.rs` 10 |
| `crates/interaction-aip/tests` | 2 | 18 | `canonical_vectors.rs` 6、`conformance.rs` 12 |
| `tests/e2e/tests` | 3 | 13 | `golden.rs` 8、`dependency_boundaries.rs` 3、`builtin_whitelist_consistency.rs` 2 |
| `crates/interaction-character-shu/tests` | 2 | 7 | |
| **合計** | | **1,278** | |

**本輪實跑**：`cargo test --workspace --no-fail-fast` → **1,278 passed／0 failed／0 ignored**，
分佈在 98 個 `test result:` 區塊（79 個測試 binary ＋ 19 組 doc-test；其中 21 個區塊是 0 passed）。
證據 `baseline/rust-test-workspace.stdout.txt`（`test result:` 行）與 `.stderr.txt`（`Running`／`Doc-tests` 行）。
靜態宣告數與實跑數在本輪**剛好相等（1,278）**——這只是這一版的事實，不是恆等式（§1.7）。

`apps/interaction-desktop/src-tauri/src`（10 檔、78 宣告）**不在 workspace**，由獨立 `cargo test --manifest-path`
執行：本輪 **78 passed／0 failed**（3 個 result 區塊，`baseline/tauri-test.stdout.txt`）。

### 1.2 前端 vitest

- `apps/interaction-desktop/src/` 下 **92 個測試檔**（全部落在 `src/test/`），宣告 **1,563** 個 `it(`／`test(`。
- **本輪實跑：92 files／1,916 tests passed，22.51 s**（`baseline/fe-test.stdout.txt`）。
  1,916 > 1,563 的差額來自 `it.each`（例：`semantic-state-contract.test.ts` 宣告 6 支、跑出 54 支；
  `canonical-vectors.test.ts` 宣告 5 支、跑出 29 支；`canonical-hash.test.ts` 宣告 15 支、跑出 24 支）。
- **17 個檔名以 `regressions-` 開頭**，合計宣告 **399** 支（約占宣告數的四分之一），依「對抗審查輪次」
  而非依行為分檔（`regressions-phase7`／`-review2-*`／`-review3-*`／`-run2-*`／`-v04`／`-v05*`／`-v06*`）。
  *（dim-I 報告寫 94 檔／13 個 regressions 檔，與本輪重查不符；以本節的 92／17 為準，且 92 與 vitest 自報的
  「Test Files 92 passed (92)」一致。）*

### 1.3 Playwright（真 daemon ＋ fixture agent ＋ Chromium）

`apps/interaction-desktop/e2e/`：**14 個 spec、92 個 `test(`**。**本輪實跑：92 passed，3.6 min**
（`baseline/pw-e2e.stdout.txt`）。單 worker、`fullyParallel:false`、`retries:0`，三段 project 依序
`first-run`（app.spec）→ `main` → `estop-last`（estop.spec）。

以下每支 spec 的秒數是**本輪 log 內每個測試自報時間的加總**（不是 timeout 設定值），用 §0.3 第四段的腳本重算：

| spec | 本輪實測 | 案例 | 行數 |
|---|---:|---:|---:|
| `general-mode-tasks.spec.ts` | 77.4 s | 20 | 1641 |
| `evidence.spec.ts` | 45.4 s | 15 | 751 |
| `character-session.spec.ts` | 22.6 s | 4 | 411 |
| `work-delegate.spec.ts` | 16.4 s | 7 | 358 |
| `app.spec.ts` | 8.4 s | 15 | 365 |
| `a11y.spec.ts` | 7.8 s | 6 | 208 |
| `sensors.spec.ts` | 5.3 s | 3 | 177 |
| `estop.spec.ts` | 5.3 s | 3 | 210 |
| `iphone.spec.ts` | 5.2 s | 4 | 217 |
| `home-state.spec.ts` | 4.6 s | 4 | 235 |
| `character.spec.ts` | 2.9 s | 3 | 110 |
| `narrow.spec.ts` | 2.0 s | 5 | 125 |
| `offline.spec.ts` | 1.6 s | 1 | 12 |
| `agent-not-installed.spec.ts` | 1.4 s | 2 | 76 |
| **合計** | **206.3 s** | **92** | |

（合計 206.3 s 對上整個 job 的 3.6 min：差額是 daemon 起停、build 與 fixture 準備。）

### 1.4 iOS XCTest（模擬器）

12 個測試檔、**163 支 `func test`**。**本輪實跑：`{"status":"passed","expected":163,"total":163,
"passed":163,"failed":0,"skipped":0}`**（`baseline/ios-simulator.stdout.txt`）。
**證據層級 `simulator`，不是 `real-iphone`。**

### 1.5 腳本、走查與演練

| 位置 | 內容 | 本輪實跑 |
|---|---|---|
| `scripts/v03-cli-e2e.sh` | 689 行；真 daemon ＋ mock 裝置 ＋ fixture agent 的 CLI 驗收 | **96 passed／0 failed**（`baseline/cli-e2e.stdout.txt` 末行 `RESULT:`） |
| `scripts/tests/architecture-checks.sh` | `CHECKS[]` 共 **18 條具名檢查**、6 組（rust 8／ts 5／docs 2／drills 1／drill-lint 1／swift 1）；本身就是一份「檢查什麼 → 可執行證據」對照表 | 六組**分別**執行、全部 PASS（`baseline/arch-{rust,ts,docs,swift,drills,drill-lint}.stdout.txt`）；`--swift` 報 native Swift 58 passed／0 failed；`--drills` 實跑四個 runner；`--drill-lint` 只做靜態檢查（6 支腳本，明寫「未實跑」） |
| `scripts/tests/docs-claims.sh`、`release-scripts.sh` | 文件宣稱 lint 與發布腳本自測（`--docs` 組會跑到） | 隨 `arch-docs` PASS |
| `scripts/tests/tauri-*.py`（4 支） | 原生 macOS AX 走查：`tauri-mobile-sensor-walkthrough.py`／`tauri-work-cancel.py`／`tauri-preset-recovery.py`／`tauri-settings-walkthrough.py`；agent 一律 fixture | **不在 baseline.sh**；本輪由真 App 走查另跑，結果見 `analysis/e2e-baseline.json`（E0-01 native completed；E0-03／E0-08／E0-11 native `harness-failed`） |
| `scripts/tests/ios-simulator.sh` ＋ `xctest-result.py` ＋ `test_xctest_result.py` | iOS 模擬器 runner 與其自測 | 見 §1.4 |
| `scripts/tests/semantic-state-swift.sh` | native macOS Swift 純模型 | 隨 `arch-swift` PASS（58 passed） |
| `scripts/drills/` | `character-package.mjs`／`restricted-device.py`／`provider-disable-reenable.sh`→`provider-lifecycle.py`／`optional-state.mjs`／**`remove-package-keep-user-data.sh`（無 runner 呼叫，見 I-17）** | 前四支隨 `arch-drills` 實跑；第五支只有 `bash -n` |
| `firmware/esp32-companion/compile.sh` | arduino-cli 編譯檢查（**不是**真板驗收） | default 與 `--ble` 兩次 exit 0（`baseline/firmware-*.stdout.txt`） |
| `scripts/tests/phase0/`（本階段新增） | `baseline.sh`／`agent_smoke.py`／`multi_session.py`／`restart_test.py`／`archive-evidence.py`／`run-step.sh` | 見 §5 |

`cargo fmt --all --check` 與 `cargo clippy --workspace --all-targets -- -D warnings` 本輪皆 exit 0
（`baseline/rust-fmt.stdout.txt`、`baseline/rust-clippy.stdout.txt`）。

### 1.6 誰真的會自動跑

`.github/workflows/ci.yml` 只有 **4 個 job**：

| CI job | 命令 | 覆蓋 |
|---|---|---|
| `rust` | `cargo fmt --check`／`clippy`／`cargo test --workspace --no-fail-fast`／`cargo build --workspace` | workspace 1,278 |
| `desktop-backend` | src-tauri 的 `clippy` ＋ `cargo test` | Tauri 78 |
| `frontend` | `pnpm aip:check`／`typecheck`／`pnpm test`／`build` | vitest 92 檔 |
| `e2e` | `pnpm exec playwright install` ＋ `pnpm test:e2e` | Playwright 92 |

`scripts/release-verify.sh` 的本機關卡另加 `docs-claims.sh` 與 `release-scripts.sh`（`--run-tests` 只是把上面
四個 job 的命令再跑一次）。因此以下**不在任何 CI job、也不在任何發布關卡**：

- `architecture-checks.sh` 的 **docs／swift／drills** 三組（rust／ts 兩組是既有 CI 的子集）。
- `scripts/v03-cli-e2e.sh`（96 個 CLI 驗收斷言）。
- iOS XCTest 163 支——**因此「三端一致」在 PR 上實際只驗 Rust ＋ TS 兩端**。
- 4 支 Tauri 原生 AX 走查——**唯一**會碰到 Tauri IPC、可信 overlay、原生檔案選擇器與 tray 的執行路徑。
- 5 支 drills、`firmware/esp32-companion/compile.sh`、`pnpm perf`。
- `scripts/tests/phase0/` 全部（§5）。

### 1.7 宣告數 ≠ 執行數 ≠ 保護力

三個獨立的量，本文件全程分開講：

1. **宣告數**（原始碼靜態計數）——本節的表格。
2. **執行數**（runner 自報）——`baseline/` 的 stdout。前端 1,563 → 1,916 是 `it.each` 展開；
   Rust 1,278 → 1,278 是巧合，不是規則。
3. **保護力**——§2 的「缺口」欄與 §3 的低價值盤點。`declarative_session_loop.rs` 的 23 個
   `Fixture::spawn() else { return; }`（I-10）證明：一支測試可以同時「被宣告」「被執行」「報 ok」，
   卻**一個斷言都沒跑**。

---

## 2. 專業測試責任矩陣

每個小節兩張表：**表 A**＝責任歸屬（風險、不變量、最適層級與理由、觸發），**表 B**＝覆蓋現況、缺口與診斷。
「現有案例」欄的 `檔名::測試名` 全部經本文件作者以 grep 逐一確認存在於 `78dcda1`。

### 2.1 安全與權限不變量（最高風險）

#### 表 A — 責任與層級

| 行為 | 風險與影響 | 要保護的不變量 | 最合適層級與理由 | 執行觸發 |
|---|---|---|---|---|
| **Policy 有效值 = min** | 任一上限被略過 → AI 拿到超過使用者偏好或裝置安全上限的強度／時長，造成實體傷害 | `effective = min(AI 請求, 使用者偏好, session 限制, 裝置安全上限, 剩餘預算)` | `unit`：governor 是確定性純函式，上 daemon 只會變慢並掩蓋輸入 | CI `rust` |
| **consent 不可由 AI 授予** | AI 自授同意 → 所有高風險動作的閘門失效 | agent／session token 不得 grant-consent、clear-estop、mutate-human-state | `contract`：授權住在 token scope 與中介層，純函式證明不了 | CI `rust` |
| **estop 不自動恢復、不謊稱全停** | 重啟自動解除 → 已停的東西又動了；謊稱全停 → 使用者停止求救 | 重啟不清除 engaged；未確認的 actuator 不計入「全部停止」；高風險能力不自動恢復 | `integration`：持久性正是要驗的東西，需真 runtime ＋ 真 SQLite | CI `rust` ＋ `e2e` |
| **誠實階梯 claimed≠verified** | 把 agent 自述當事實 → 使用者以為工作完成 | queued≠completed；acknowledged≠completed；completed≠verified；未知一律 `uncertain` | `integration`(fixture 子程序) ＋ `contract`(HTTP) ＋ `unit`(前端投影) | CI `rust` ＋ `e2e`；`real-agent` 不在 gate |
| **lease 到期即失效** | 過期 session 仍能用能力 → 授權無限期延長 | 到期 → 撤銷能力、拒絕續期、發 timed-out | `integration`：fixture 子程序即可 | CI `rust` |
| **取消要殺整棵程序樹** | 只殺 leader → 子程序繼續改檔案／燒額度 | 取消／estop 後 process group 死亡，不因 leader 先結束而漏殺 | `integration`：必須真 fork／真 signal，mock 沒有意義 | CI `rust`（Linux） |
| **重啟後未完成工作是 unknown** | 重啟把進行中講成完成或失敗 | 開著的 session 不跨重啟存活；未完成一律 unknown | `integration`：真 SQLite ＋ 真 home，in-memory 會讓測試失效 | CI `rust`（優雅重啟）；硬殺不在 gate |
| **session 隔離** | 兩 session 串線 → A 的輸出／mailbox 進到 B，或取消 A 連帶殺 B | 各自獨立 workdir／mailbox／provider 程序／token；取消一個不影響其他 | `integration`(fixture 即可，隔離是 runtime 責任)；真 agent 只是加強證據 | **自動化：無**；`real-agent` 只在 `phase0/multi_session.py` |
| **mailbox 有界** | 無界 queue → 記憶體爆或 agent 被灌爆 | `maxMessages` 是硬上限；0 不等於無限 | `integration`：fixture 即可 | CI `rust` |
| **Context Bundle 內容** | 塞進不該給 AI 的東西 → 隱私外洩；靜默截斷 → AI 拿到殘缺脈絡卻不自知 | session-scoped、決定性、截斷要說出來 | `integration`：決定性靠真 SQLite 的真排序 | CI `rust` |

#### 表 B — 覆蓋現況、缺口與診斷

| 行為 | 覆蓋狀態 | 現有案例（本文件已逐一 grep 確認存在） | 缺口（`static-inspection`） | 預期結果與診斷方式 |
|---|---|---|---|---|
| **Policy 有效值 = min** | `implemented-and-connected` | `crates/interaction-policy/src/lib.rs`（全檔 10 支）：`::effective_limit_is_min_of_all_caps`、`::session_budget_clamps_then_blocks`、`::pattern_bounding_clamps_steps_and_magnitude`、`::cooldown_and_hourly_limits`、`::quiet_hours_block_audio_but_not_conversation`、`::unlisted_actuator_is_blocked`、`::autonomous_respects_initiative` | 10 支守 5 條上限的交叉組合；沒有 property／組合式測試；`min_opt_*` 系列輔助函式的 None／NaN 邊界沒有獨立案例 | 斷言訊息要印出 `effective` 與五個輸入上限，讀者才看得出是哪一條沒夾住 |
| **consent 不可由 AI 授予** | `implemented-partial` | `crates/interaction-api/tests/api_e2e.rs::agent_token_cannot_grant_consent_clear_estop_or_mutate_human_state`、`::only_a_human_token_can_verify_a_claim_and_no_agent_can_self_upgrade`、`::mobile_routes_are_human_only_for_agent_and_session_tokens`、`::character_session_routes_are_human_only`；`real-agent` 佐證見 `analysis/e2e-baseline.json` E0-10（agent token 全部確定性阻擋） | **CLI 端無對應測試**：`crates/interaction-cli/tests`（18 支）沒有一支驗「CLI 帶 agent token 不能授予同意」。「誰能授予」只在 HTTP 一端有覆蓋 | 斷言 HTTP status code ＋ error code，**不看文案**（文案改寫不該讓安全測試紅或綠） |
| **estop 不自動恢復、不謊稱全停** | `implemented-partial` | `crates/interaction-runtime/tests/estop_parallel.rs::an_emergency_stop_with_unconfirmed_actuators_never_claims_every_output_halted`（該檔共 4 支）；`character_session_loop.rs::an_engaged_emergency_stop_is_replayed_into_the_session_on_startup`；`providers_loop.rs::operational_provider_does_not_auto_recover_after_restart`、`::tested_evidence_survives_restart_but_never_re_arms_the_device`、`::a_revoked_declarative_device_stays_off_across_a_restart`；`agents_loop.rs::estop_cancels_all_open_sessions_and_blocks_new_ones`；`e2e/estop.spec.ts`（3 案，`browser`） | 沒有**單一端到端**的「estop 中直接重啟 daemon，斷言仍 engaged 且無 actuator 被重新 arm」——現況被拆成 session replay ＋ provider not-auto-recover 兩半。`e2e/estop.spec.ts` 不重啟 daemon | 比對重啟前後的 `status.emergencyStop` 與**每一個** provider 的 `state`；不確認的 actuator 必須出現在「未確認」清單裡 |
| **誠實階梯 claimed≠verified** | `implemented-and-connected`（fixture 範圍）／`implemented-partial`（真 agent） | `agents_loop.rs::delegation_honesty_ladder_dispatched_acknowledged_claimed`、`::human_verify_is_the_only_path_from_claim_to_verified`、`::a_claim_never_auto_upgrades_itself_to_verified`；`gateway_loop.rs::process_that_ends_without_a_claim_is_unknown_and_only_a_real_error_is_failed`、`::working_never_precedes_the_task_actually_reaching_the_agent`；`api_e2e.rs::an_unknown_outcome_is_reportable_and_can_never_be_verified`；`src/test/honesty.test.ts`（8 支）；`e2e/work-delegate.spec.ts`（7 案，含「綠勾只有按下標記為已驗證才出現」） | (a) fixture 的失敗模式是有限集（silent／crash／deaf／hang／claim-then-crash／claim-then-silent），真 agent 的部分輸出、串流中斷、rate-limit、context 溢出沒有 fixture；(b) `GatewayEvent::TaskWaitingForInput` 在 `crates/interaction-agent-gateway/src/lib.rs:99` 有定義，但整個 gateway 沒有任何 connector 產生它（`grep -rn TaskWaitingForInput crates` 只有定義處與 `runtime/src/gateway.rs:413` 的消費端）→ 這一態是 `defined-only` | 斷言 **state 序列 ＋ 事件序列**，不看文案。真 agent 的對照證據見 `analysis/e2e-baseline.json` E0-02／E0-09 |
| **lease 到期即失效** | `implemented-and-connected` | `agents_loop.rs::lease_expiry_kills_capabilities_and_refuses_renewal`、`::lease_expiry_emits_timed_out_and_revokes_the_session_capability`；`crates/interaction-session/tests/session_hardening.rs`（17 支） | 沒有「lease 到期時**子程序也被殺**」的案例——只驗到能力撤銷，程序樹存活與否未斷言 | `state == TimedOut` ＋ `renew` 回錯；補的話再加 `pid_alive()` 輪詢 |
| **取消要殺整棵程序樹** | `implemented-and-connected`（Unix） | `crates/interaction-agent-gateway/src/process.rs`（4 支）：`::kill_tree_escalates_to_group_even_when_leader_exits_in_grace`、`::kill_tree_signals_group_even_after_leader_reaped`、`::process_group_terminate_kills_sigterm_ignoring_members`、`::delegated_process_never_inherits_runtime_capability_tokens`；`gateway_loop.rs::estop_terminates_sessions_concurrently`、`::an_interrupted_codex_turn_is_cancelled_not_claimed_completed` | 只在 Unix（`process.rs` 走 `libc`），Windows 路徑零測試。真 agent 自己 spawn 的孫程序沒有自動化覆蓋；`real-agent` 取消只在 `phase0/agent_smoke.py --cancel-when-active`，且本輪在 claude 端**發現產品缺陷**（見 §2.1 註） | `pid_alive()` 輪詢 ＋ 逾時訊息要印出還活著的 pid／pgid |
| **重啟後未完成工作是 unknown** | `implemented-partial` | `agents_loop.rs::restart_reports_unknown_for_work_that_was_still_open`、`::open_sessions_do_not_survive_restart`；`sensors_loop.rs::restart_retains_unresolved_without_claiming_active_capture` | 全部是**優雅重啟**（drop 再 start）。SIGKILL／斷電只有 `phase0/restart_test.py`（`real-agent`，不在 gate）；SIGTERM 路徑本輪未跑（`analysis/e2e-baseline.json` E0-04 改判 `harness-failed`） | 比對重啟前後的 session state；孤兒程序用 ppid／pgid 鏈過濾後斷言 |
| **session 隔離** | `absent`（自動化）／`implemented-partial`（`real-agent`，不在 gate） | 相關但不等價：`gateway_loop.rs::estop_terminates_sessions_concurrently`（開兩個 session，但斷言的是 estop 併發性）、`::workdir_never_exposes_the_runtime_state_dir`、`::a_symlink_cannot_smuggle_the_runtime_state_dir_in_as_a_workdir`、`::resume_cannot_widen_scope`；`api_e2e.rs::an_agent_session_can_fetch_its_own_mailbox_and_that_is_what_marks_delivery`、`::interrupt_requires_session_ownership_or_a_human_token`。**真 agent 證據**：`analysis/e2e-baseline.json` E0-06／E0-07 `completed`（claude×2、codex×2、claude＋codex 各一輪，0 筆 SSE sessionId 混淆、無暗號交叉、取消不干擾另一個），原始資料 `real-agent-e2e-prior/multi-C-claude-x2/`、`multi-D-codex-x2/`、`multi-E-claude-codex/` | 全 repo 找不到**單一自動化測試**同時斷言 (a) A 的 mailbox 不會被 B fetch、(b) 取消 A 後 B 仍 active、(c) A 的輸出不寫進 B 的 record。這是矩陣裡唯一「安全不變量 ＋ 自動化零覆蓋」的格子 | fixture 就夠：兩個 session 各寫獨一暗號到 `fake-input`，交叉比對 mailbox 集合互斥、B 的 state 未變 |
| **mailbox 有界** | `implemented-and-connected` | `agents_loop.rs::message_budget_is_a_hard_ceiling`、`::max_messages_zero_does_not_mean_unlimited`；`api_e2e.rs::oversized_payload_is_rejected`（單則 byte 上限） | 覆蓋良好；byte 上限只在 HTTP 層驗，runtime 層未獨立驗 | 斷言被拒的那一則的錯誤碼與 queue 長度 |
| **Context Bundle 內容** | `implemented-partial` | `agents_loop.rs::every_task_receives_and_persists_the_exact_session_scoped_context_bundle`；`memory_loop.rs::context_bundle_is_deterministic_and_honest`、`::context_bundle_reports_capacity_truncation`；`real-agent` 佐證 `analysis/e2e-baseline.json` E0-05（bundle 送達且被 agent 讀到） | 沒有**否定式**測試：「bundle 內不得出現 secret、不得出現其他 session 的記憶」。`memory_loop.rs::secrets_in_tags_and_provenance_are_rejected` 守的是寫入端，不是讀出端 | 斷言 bundle 的 canonical 序列化**逐位元組**相同；否定式測試要印出命中的字串 |

> **§2.1 註（取消程序樹的真 agent 缺陷）**：本輪 `real-agent` 測到人類 interrupt 真 claude-code session
> 的終態是 `failed` 而非 `cancelled`（5/5 重現），Claude 連接器在任何路徑都產不出 `cancelled`
> （`grep -c TaskCancelled crates/interaction-agent-gateway/src/claude.rs` == 0）。詳見
> `analysis/e2e-baseline.json` 的 D1，以及同批交付的 `phase-0-e2e-baseline.md`。
> 現有自動化契約 `gateway_loop.rs::an_interrupted_codex_turn_is_cancelled_not_claimed_completed`
> **只有 codex 一端有測試**——這正是「測試層級選對了、但只鋪了一半」的具體例子。

### 2.2 狀態、持久化與相容性

#### 表 A — 責任與層級

| 行為 | 風險與影響 | 要保護的不變量 | 最合適層級與理由 | 執行觸發 |
|---|---|---|---|---|
| **memory TTL／刪除** | 過期記憶仍供給 AI；使用者刪不掉 | 過期即拒絕且被 sweep；agent 建立的 user-memory 不得超 30 天；刪除要真刪 | `integration`：TTL 與 cascade 都是 SQL 行為 | CI `rust` |
| **snapshot 遷移** | 舊快照誤判毀損 → session 全丟；未來版被覆寫 → 降版即資料損毀 | 舊格式遷移、未來格式原樣保留、截斷者隔離換 epoch | `contract`（golden fixture）：不需 daemon，但需真的舊 bytes | CI `rust`；`architecture-checks --rust` 另列為 `snapshot-migration` |
| **receive 決策表三端一致** | 三端對同一封 envelope 做不同決策 → 桌面／手機／runtime 狀態不一致 | 同一份 fixture、同一個決策 | `contract`：唯一能證明跨語言一致的層級 | Rust／TS 在 CI；**Swift 不在** |
| **canonical JSON／state hash 三端一致** | hash 對不上 → 桌面無法核對 host 狀態，那條防線等於不存在 | 逐位元組相同的 canonical 文字與 SHA-256 | `contract`：向量由 Rust 產生，TS／Swift 只准對答案 | Rust／TS 在 CI；**Swift 不在** |
| **SSE 重連／Last-Event-ID** | 游標錯 → 事件被吞或舊事件重播（畫面冒出過時綠勾） | 初次連線從目前序號起；同 daemon 續用 lastId；daemon 重啟則重置 | `contract`(HTTP) ＋ `unit`(游標純函式 ＋ stub fetch 驗真實 request) | CI `rust` ＋ `frontend` |
| **設定恢復（兩段寫入中斷）** | 套用檔位中途失敗 → 畫面只剩「自訂」讓使用者猜；或用過時意圖覆蓋剛改的設定 | 任一段失敗都要說出來、可只補送第二段；重開後 marker 還在且使用者沒改過才自動補送 | `unit`(前端投影) ＋ `unit`(Tauri 服務) ＋ `native-desktop` 走查（唯一能證明真 IPC ＋ 真 WebView 走得通） | 前端在 CI `frontend`、`preset_service` 在 `desktop-backend`；**native 走查不在** |
| **備份還原** | 還原寫一半卻宣稱成功；使用者以為「備份」＝完整備份 | 拒絕就是一筆都不寫；中途失敗要說已寫入幾筆；不得自稱完整備份 | `contract`（應該要有：真 daemon 種資料 → export → 換 home → 還原 → 比對）；現況只有 `unit`(mock) | CI `frontend`(mock) ＋ `e2e`(只驗按鈕存在) |

#### 表 B — 覆蓋現況、缺口與診斷

| 行為 | 覆蓋狀態 | 現有案例 | 缺口（`static-inspection`） | 預期結果與診斷方式 |
|---|---|---|---|---|
| **memory TTL／刪除** | `implemented-partial` | `memory_loop.rs::expired_memories_are_pruned_by_sweep`、`::expired_memories_are_refused_before_sweep`、`::agent_user_memory_cannot_be_extended_beyond_thirty_days`、`::secrets_in_tags_and_provenance_are_rejected`（該檔 15 支）；`knowledge_loop.rs`（14 支，含刪除 cascade） | 沒有「刪除後 SQLite 真的無殘留（含 FTS index、衍生表）」的測試，只驗 API 讀不到。**本輪 `real-agent` 另外測到一件更根本的事**：記憶刪除後，agent 端 `resume` 仍能完整覆誦已刪除的機密暗號（`analysis/e2e-baseline.json` E0-05，E-09）——「刪除」在 provider transcript 上不可回收 | 直接查表計數，不要只查 API |
| **snapshot 遷移** | `implemented-and-connected` | `character_session_loop.rs::a_v0_6_0_snapshot_is_restored_and_migrated_to_the_current_format`、`::a_snapshot_from_before_unsupported_intents_is_migrated_instead_of_quarantined`、`::a_future_format_snapshot_is_kept_untouched`、`::a_truncated_snapshot_is_quarantined_with_a_new_epoch`；`crates/interaction-character/tests/migration_registry.rs`（10 支） | fixture 是**內嵌字串**而非凍結檔案：每多一版就要多一段手寫 fixture，沒有機制強制新增。`src/test/semantic-state-contract.test.ts` 用 `releases/v0.7.0/` 凍結 corpus ＋ `sourceSha` 鎖定，是更好的模式 | 斷言遷移後的 canonical 與 epoch；隔離時要斷言「換了新 epoch」而不是「沒爆炸」 |
| **receive 決策表三端一致** | `implemented-partial`（三端有測、只有兩端在 gate） | Rust `crates/interaction-session/tests/receive_decision_fixtures.rs`（3 支）、`receive_decisions.rs`（8 支）、`receive_decisions_from_json.rs`（2 支）；TS `src/test/receive-decision-fixtures.test.ts`（2 支 `it.each`）；Swift `ReceiveDecisionConformanceTests`（隨 iOS 163 支一起跑，`simulator`） | **Swift 端不在 CI**：`ios-simulator.sh` 與 `architecture-checks --swift` 都不在 `ci.yml`，所以「三端一致」在 PR 上只驗兩端 | 逐筆列 fixture id 的 diff，不要只印「不相等」 |
| **canonical JSON／state hash 三端一致** | `implemented-partial`（同上） | Rust `crates/interaction-aip/tests/canonical_vectors.rs`（6 支）、`conformance.rs`（12 支）；TS `src/test/canonical-hash.test.ts`（宣告 15／實跑 24）、`canonical-vectors.test.ts`（宣告 5／實跑 29）、`semantic-state-contract.test.ts`（宣告 6／實跑 54，含 `releases/v0.7.0/` 凍結 corpus）；Swift `CanonicalVectorsTests`／`StateHashConformanceTests`（`simulator`）＋ native `semantic-state-swift.sh`（58 passed，在 `arch-swift`） | 同上，Swift 不在 CI。這仍是全 repo 品質最高的一組：向量由 Rust 產生、其他端只准對答案 | 逐位元組 diff ＋ 印出兩端的 canonical 文字 |
| **SSE 重連／Last-Event-ID** | `implemented-partial` ＋ 一處 `confirmed-defect`（見 I-04） | `api_e2e.rs::sse_stream_replays_with_last_event_id`；`src/test/regressions-v05-round2.test.tsx` 的 `nextStreamCursor` 純函式案例 | **接線本身只有原始碼字串比對**（I-04）：沒有任何測試用 stub fetch 斷言 `runStream` 真的送出 `Last-Event-ID` header。`src/test/transport.test.ts`（2 支）已示範可行寫法 | 斷言第一個 fetch 打 `/v1/status`、第二個 fetch 的 `headers["Last-Event-ID"]`；再模擬 daemon 重啟斷言游標重置 |
| **設定恢復（兩段寫入中斷）** | `implemented-and-connected`（前端＋Tauri 服務）／`implemented-partial`（native 走查不在 gate） | `apps/interaction-desktop/src-tauri/src/preset_service.rs`（11 支）；`src/test/companion-preset-recovery.test.tsx`（16 支）、`companion-preset-a11y.test.tsx`（9 支）、`apply-preset-plan.test.ts`（13 支）；`scripts/tests/tauri-preset-recovery.py`（211 行，真 App ＋ 隔離 loopback proxy 注入故障）。**本輪 native 實跑**：10/10 故障案例收斂、113.66 s（`analysis/e2e-baseline.json` E0-01，證據 `e2e-runs/R7/preset/result.json`） | native 走查不在 CI；且本輪走查用的是 **2026-09-06 的 0.7.0 bundle，不是 HEAD build**（磁碟不足無法重建）→ 這一列的 `native-desktop` 證據**不綁在 `78dcda1`** | marker 檔 ＋ 兩端讀回值；故障注入放 loopback proxy（不動 production flag）是對的設計，保留 |
| **備份還原** | `implemented-partial`（匯出）／`needs-investigation`（還原） | `src/test/backupSection.test.tsx`（7 支，全部對 `vi.spyOn(api, …)` 的 mock）；`e2e/app.spec.ts` 只驗按鈕存在。**本輪實測**：匯出（API 與 CLI）與「壞資料還原被擋（400）」在 API／CLI 路徑 `completed`／`correctly-blocked`；native 匯入 `harness-failed`（`analysis/e2e-baseline.json` E0-11） | **矩陣裡最大的結構缺口**：後端只有 `GET /v1/memory/export`（`crates/interaction-api/src/lib.rs:193`），**沒有 import／restore 路由**；還原是前端對每一筆呼叫 `memoryCreate` 的迴圈。因此 (a) 零 round-trip 自動化測試；(b) 中途失敗的原子性只在 mock 上驗過。E0-11 的 native 走查另有一個更嚴重的發現：AX `choosefile` 從不驗證真的選到哪個檔 → 2026-09-06 那次綠燈可能是假陽性 | 應加 `contract`：真 daemon 種資料 → export → 換 home → 逐筆還原 → 斷言 `memory_status` 一致。`scripts/v03-cli-e2e.sh` 是它自然的家 |

### 2.3 裝置、感測與呈現

#### 表 A — 責任與層級

| 行為 | 風險與影響 | 要保護的不變量 | 最合適層級與理由 | 執行觸發 |
|---|---|---|---|---|
| **iPhone 配對／撤銷即斷線** | 撤銷後手機仍連著 → 已收回的授權還在生效 | 配對碼錯即燒毀；撤銷立即斷線且舊 token 再也認證不過；斷線＝能力不可用 | `integration`（真 WebSocket ＋ TLS ＋ fixture 手機）：協定行為必須真的跑過 | CI `rust` ＋ `e2e`；iOS 端**不在** |
| **感測停止結果誠實投影** | 說「已停止」但沒停 → 使用者以為麥克風關了 | 只有重讀 activeSensors 為空且無不確定才可說已停止；查詢失敗說查詢失敗；歷史未解決要留提醒；不外洩原始 id | `unit`(投影純函式) ＋ `integration`(停止路徑) ＋ `browser`(橫幅真的出現) | CI `rust` ＋ `frontend` ＋ `e2e` |
| **裝置 rebind／撤銷不復活** | 停用後自己接回來 → 使用者關掉的裝置還在動 | 免重啟 rebind、世代拒絕遲到回呼、撤銷中不復活、逾時有界 | `simulator`（pty／MQTT 模擬器）：真板不可進 CI，但線協定要真的跑過 | CI `rust`——**但見 I-10：無 python3 時整組靜默變綠** |

#### 表 B — 覆蓋現況、缺口與診斷

| 行為 | 覆蓋狀態 | 現有案例 | 缺口 | 預期結果與診斷方式 |
|---|---|---|---|---|
| **iPhone 配對／撤銷即斷線** | `implemented-and-connected`（fixture）／真機 `needs-investigation` | `crates/interaction-runtime/tests/mobile_loop.rs`（81 支）含 `::wrong_pairing_code_is_refused_and_session_burned`、`::token_reconnect_works_until_revoked`、`::revoke_disconnects_live_connection_immediately`、`::idle_connection_times_out_and_capabilities_go_offline`、`::high_risk_receptor_forced_off_on_disconnect_and_stays_off`；`e2e/iphone.spec.ts`（4 案）；`ConnectionManagerGateTests`（`simulator`）。**本輪 fixture 實測** `completed`（`analysis/e2e-baseline.json` E0-08） | **真機（`real-iphone`）本階段 0 筆**：`analysis/e2e-baseline.json` 記 `needs-environment`——iPhone 11 已連線且 Developer Mode 已開，但 Xcode 的 IDEProvisioningTeams 為空，`device-build.sh --check-only` 卡在 step 3/5（需人類在 Xcode 選 Team）。另：出貨 fixture `fake_iphone` 在連線被拒時 `process::exit(2)`，本輪改用自寫 WS probe 才驗到 daemon 側行為 | WebSocket 關閉碼 ＋ `mobileStatus` 的 device 狀態；撤銷後要斷言舊 token **被拒**，不是只斷言連線消失 |
| **感測停止結果誠實投影** | `implemented-and-connected` | `src/test/sensorStop.test.ts`（18 支，含「裝置沒回覆：結果不確定」`:99`、「讀不到狀態：說無法確認，不猜成功也不猜失敗」`:180`、「認不得的種類不猜、也不外洩原始 id」`:18`）、`unresolvedStops.test.tsx`（17 支）、`sensor-stop-history.test.tsx`（1 支，走真 App→Shell→SensorBanner）；`sensors_loop.rs`（38 支，含 `::every_stop_outcome_of_a_source_is_reported_and_evented_honestly`——逐一走過 confirm→`stopped`／timeout→`unknown`／unreachable→`unreachable`／refuse→`refused` 四種來源回應、`::orphan_ttl_moves_unknown_to_unresolved_not_to_normal`、`::the_stop_sweep_is_bounded_by_the_deadline`）；`sensor_journal_review.rs`（5 支）；`e2e/sensors.spec.ts`（3 案） | 全 repo 最紮實的一塊。唯一缺口是真麥克風：`adapters/media` 的 cpal 路徑只有 `FakeSource`，真裝置行為零自動化（**合理**——真收音不該進 CI） | 斷言投影的 `ok`／`message` **分支**，不是文案全文；橫幅測試要斷言「未解決」提醒不會被清掉 |
| **裝置 rebind／撤銷不復活** | `confirmed-defect`（機制層）／`implemented-and-connected`（有 python3 時） | `declarative_session_loop.rs::reenable_rebinds_without_restart`、`::rebind_generation_rejects_late_callbacks`、`::revoke_during_rebind_does_not_resurrect`、`::rebind_timeout_is_bounded_and_honest`（`scripts/tests/architecture-checks.sh:55` 明列這四支為 `adapter-lifecycle` 的唯一可執行證據）；`mqtt_rebind_loop.rs`；`providers_loop.rs::re_enabling_a_declarative_device_rebinds_without_a_restart`、`::a_legacy_disabled_provider_without_the_off_marker_stays_locked_after_restart` | **I-10（confirmed）**：`declarative_session_loop.rs` 有 **23 處** `let Some(..) = Fixture::spawn() else { return; }`；無 python3 時 `Sim::spawn_with` 回 `None`（`:66-70`），這些測試直接 return 並報 `ok`。**ESP32 真板驗收 0 筆**（`real-hardware` = 0；`compile.sh` 只是編譯檢查） | 模擬器 log ＋ `members()`；**首要修法**是把 `python3_available()` 的 `return None` 改成 panic，repo 內已有先例 `crates/interaction-session/tests/state_hash_fixtures.rs::a_schema_too_deep_to_walk_panics_instead_of_skipping_silently` |

### 2.4 介面、可及性與角色呈現

#### 表 A — 責任與層級

| 行為 | 風險與影響 | 要保護的不變量 | 最合適層級與理由 | 執行觸發 |
|---|---|---|---|---|
| **五入口可達／不外洩技術詞** | 一般模式漏出 UUID、`provider.mobile.<id>`、revision → 使用者看不懂，也洩漏內部識別碼 | 五個一級入口都可達；一般模式渲染後的 DOM 不得命中技術詞正規式 | `unit`(渲染後掃 DOM，比 E2E 便宜且更全面) ＋ `browser`(五入口真的到得了) | CI `frontend` ＋ `e2e` |
| **a11y（鍵盤、焦點、SR 名稱）** | 鍵盤到不了緊急停止、對話框關不掉 → 安全操作對部分使用者不可達 | 第一個 Tab 跳到主要內容；從頁首 Tab 得到緊急停止；Escape 收得掉且焦點不外逃；390px 一樣有名字 | `browser`（焦點順序只有真瀏覽器算數）＋ `unit`（焦點陷阱純邏輯） | CI `e2e` ＋ `frontend` |
| **角色呈現層沒有權限主權** | Character Pack 改寫安全文字或偽造 verified | 安全語句固定；truthState／verified 只由 Runtime 決定；不支援就誠實降級 | `unit` ＋ `contract`：協商是純函式，主權是 envelope 契約 | CI `rust` ＋ `frontend` |

#### 表 B — 覆蓋現況、缺口與診斷

| 行為 | 覆蓋狀態 | 現有案例 | 缺口 | 預期結果與診斷方式 |
|---|---|---|---|---|
| **五入口可達／不外洩技術詞** | `implemented-and-connected` | `src/test/general-mode-no-technical-terms.test.tsx`（9 支：掃一般模式**真的渲染出來的 DOM 文字**，含 `generation`／`sourceId` 兩個新外洩面；最後一案**反向**要求進階模式要出現診斷，避免退化成「刪掉就綠」）；`regressions-v06-general-mode.test.tsx`（10 支）、`general-mode-metrics.test.tsx`（15 支）；`e2e/general-mode-tasks.spec.ts`（20 案 = 13 支任務測試涵蓋 14 項任務 ＋ 7 支可及性） | DOM 全文掃描目前只覆蓋「同步卡 ＋ 未解決停止」兩個元件；工作／記憶／活動頁沒有同樣的掃描 | 失敗時要**印出命中的字串**，否則讀者不知道漏了什麼 |
| **a11y** | `implemented-partial` | `e2e/a11y.spec.ts`（6 案，本輪 7.8 s）；`e2e/general-mode-tasks.spec.ts` 的 7 支「可及性：…」（`:1318`–`:1568`）；`e2e/narrow.spec.ts`（5 案）；`src/test/dialog.test.tsx`（7 支）、`global-search-a11y.test.tsx`（4 支）、`companion-preset-a11y.test.tsx`（9 支） | **沒有任何自動化 a11y 掃描器**：`grep -rn axe` 在 `src`／`e2e`／`package.json` 零命中（`pnpm-lock.yaml` 只有 `saxes`，不是 axe）。對比度只有一條手寫檢查（`src/test/regressions-review3-ia.test.tsx:43`，對 CSS 字面 hex 算 WCAG，只涵蓋 `.companion-sensor-label` 一個 class）。色彩對比、ARIA 正確性、label 關聯、landmark 結構皆無系統性覆蓋 | 焦點與鍵盤必須 `browser`；靜態規則建議用 axe 對五入口各跑一次（成本極低），失敗時印出違規節點的 selector |
| **角色呈現層沒有權限主權** | `implemented-and-connected` | `crates/interaction-session/tests/security_matrix.rs::renderer_capability_spoofing_only_earns_unsupported`、`::every_result_envelope_validates_and_never_claims_verified`、`::the_pipeline_order_is_fixed_identity_before_membership_before_scope`（該檔 7 支）；`crates/interaction-character/tests/negotiation.rs`（16 支）、`gateway.rs`（37 支）；`src/test/packs.test.ts`（9 支）、`character-gateway.test.ts`（42 支）、`adapter-contract.test.ts`（11 支） | 覆蓋良好，本文件不提出缺口 | 斷言 envelope 的 `truthState`／`verified` 欄位由 Runtime 寫入，adapter 端改不動 |

---

## 3. 低價值測試盤點

**「低價值」的定義**：維護成本或誤紅風險高於它擋住的缺陷，**或**目標缺陷真的發生時它不會失敗。
本節每一條都附 `file:line`，並經本文件作者在 `78dcda1` 上開檔重看過。
**本階段不刪除、不改寫任何一支測試**——這裡只是盤點，處置在 §4。

證據層級全部為 `static-inspection`，唯一例外是 I-18（用本輪 `browser` 實跑的時間資料改判）。

### 3.1 鎖死非契約的原始碼／CSS 文字（改一行格式就誤紅，真壞掉不紅）

| id | file:line | 為什麼低價值 |
|---|---|---|
| **I-01** | `apps/interaction-desktop/src/test/characterPage.test.tsx:778-786` | 把 `src/styles.css` 讀成字串，用 `expect(block).toMatch(/\.character-cards \{ grid-template-columns: 1fr; \}/)` 鎖死**單行 CSS 格式**（對應 `src/styles.css:743`）。`vitest.config.ts` 的 environment 是 jsdom，**不套用 CSS 也不跑 layout**：把規則重排成多行就誤紅而版面沒變；反過來，class 在 CSS 裡存在但 TSX 沒套到元素（selector 失效）它偵測不到 |
| **I-02** | `apps/interaction-desktop/src/test/characterPage-first-screen.test.tsx:639-654` | 同一手法，而且**拿 styles.css 裡兩句中文註解的子字串當書籤**（`css.indexOf("角色頁（CompanionPage）")`、`css.indexOf("H 區塊結束")`）。改註解措辭 → `indexOf` 回 `-1` → `slice(-1, x)` 不拋錯，**靜默切出錯誤區段**，結果不可預期。同一支裡「不得有 transition／animation」的意圖是有價值的，只是驗錯了層 |
| **I-03** | `apps/interaction-desktop/src/test/firstSuccess.test.tsx:189-197` | 測試名是「390px：選項是整寬的直排按鈕（class），CSS 有窄視窗規則」，卻在 `:196` 逐字比對 `.first-success .onboarding-panel { padding: 16px 12px 12px; }`（`src/styles.css:748`）。padding 數值與測試意圖脫鉤：純視覺微調就紅，按鈕真的壞掉不紅 |
| **I-04** | `apps/interaction-desktop/src/test/regressions-v05-round2.test.tsx:70-79` | `import TRANSPORT_SOURCE from "../transport.ts?raw"` 之後全用 `indexOf`／`toContain` 比對原始碼文字，包括 `toContain('"Last-Event-ID": cursor.lastId')`。改名、換引號、抽常數都會誤紅；而「呼叫了 `nextStreamCursor` 卻忽略 `reset`」這種真缺陷抓不到。同目錄 `transport.test.ts:9-33` 已示範 stub fetch 驗真實 request 的寫法 |
| **I-05** | `apps/interaction-desktop/src/test/general-mode-metrics.test.tsx:219-246` | vitest 用 `readFileSync("e2e/general-mode-tasks.spec.ts")` 讀**另一支測試的原始碼**，`indexOf('} else {')` 切分支再 `toContain('actual: "not-run"')`。e2e spec 的無害重構（改大括號風格）即誤紅；而分類條件本身寫錯（永遠走同一支）它也不會紅。`e2e/taskMetrics.ts` 目前刻意只放純計數器，把分類抽成具名函式與現有風格一致 |
| **I-06** | `apps/interaction-desktop/src/test/deep-links.test.tsx:245-249` | **原判 refuted，本表以改判為準（詳見附錄 A）**：`expect(app).toContain("onNavigate((t) => goTo(t))")` 逐字鎖死 `src/App.tsx:257` 的一次函式呼叫。**原判引用的「App.tsx 另有六處等價寫法」是錯的**——`App.tsx:344/453/513/525/539` 是 JSX 屬性語法、傳給五個不同子元件各自的 `onNavigate` prop，與 `./desktop` 匯出的模組函式只是同名，語法上不可互換。**成立的、窄化後的理由**：`App.tsx:513`／`:539` 本身就用 `onNavigate={goTo}` 這種不加箭頭包裝的風格，因此把 `:257` 簡化成同語意的 `onNavigate(goTo)` 在本檔既有風格下是合理重構，卻會讓這支 exact-string 測試誤紅 |
| **I-07** | `apps/interaction-desktop/src/test/companion-gateway-wiring.test.ts:263-283` | 用 `indexOf` 切三個檔（`CompanionApp.tsx`／`api.ts`／`src-tauri/src/character_bridge.rs`）的原始碼片段，證明 reducedMotion 的三層接線。最弱的一段是 `expect(bridge).toContain('.get("reducedMotion")')`：只要那個子字串還在檔案裡（哪怕變成死碼或讀進一個沒被轉交的變數），測試照樣綠，而值已經在中間掉了。懷疑者另查證 `crates/interaction-api/tests/api_e2e.rs` 對 `reducedMotion`／`reduced_motion` grep **零命中**——建議的 contract 補法確實還不存在 |

### 3.2 斷言太弱／鎖死內部細節（目標缺陷發生也不會失敗）

| id | file:line | 為什麼低價值 |
|---|---|---|
| **I-08** | `apps/interaction-desktop/src/test/companion-presets.test.ts:53-58` | 測試名宣稱「每個預設都有人看得懂的名稱與一行說明」，實際只斷言 `def?.label.length > 0` 與 `def?.summary.length > 4`。任何 5 字元字串都會過（含 `"TODO!"` 或原始 id）。同檔上方對 `describeCompanionState` 已有逐句硬字串斷言，證明更強的寫法在同一個檔案裡就有先例 |
| **I-09** | `apps/interaction-desktop/src/test/companion-presets.test.ts:76-79` | `expect(Object.keys(mod)).not.toContain("applyCompanionPreset")` 鎖死一個**已刪除的函式名**。`src/companion/presets.ts` 現在完全沒有 `apply*` 匯出，真正的套用入口在 `applyPresetPlan.ts` 的 `beginPresetOp`。它只擋「原樣重新引入這一個識別字」，任何改名的第二入口照樣綠——不是「永遠不可能失敗」，但**任何改名都能繞過** |
| **I-15** | `apps/interaction-desktop/e2e/helpers.ts:56-58` | `PAGES` 把角色頁導覽 label 寫死 `"小樞"`、marker 寫死 `/36 表情預覽/`（該 marker 只在 `src/pages/character/CharacterPreview.tsx` 的 shu-rig 入口渲染）。**helpers.ts 的註解自己承認**（`:54-55`）：換預設角色或索引載入失敗時，導覽會顯示中性的「角色」。屆時 14 支 spec 中約 10 支（凡是用 `navigateTo` 的）會一起紅，而錯誤訊息指向導覽 marker 不符，不是真因 |

### 3.3 永久／靜默 skip（綠燈不代表跑過）

| id | file:line | 為什麼低價值 |
|---|---|---|
| **I-10** | `crates/interaction-runtime/tests/declarative_session_loop.rs:66-70` | `Sim::spawn_with` 在無 python3 時 `eprintln!` 後回 `None`；全檔 **23 處** `let Some(..) = Fixture::spawn() else { return; };`（例：`:342`、`:1551`）直接 return 成功。該檔 2,696 行、33 支測試，並涵蓋 `scripts/tests/architecture-checks.sh:55` 明列的 `adapter-lifecycle` 四條關卡。**缺 python3 時 `cargo test` 報 ok，發布關卡看不出任何差別。** repo 內已有相反紀律的先例：`crates/interaction-session/tests/state_hash_fixtures.rs::a_schema_too_deep_to_walk_panics_instead_of_skipping_silently` |
| **I-11** | `crates/interaction-adapter-declarative/tests/esp32_sim_conformance.rs:66-70` | 同一個 python3 靜默 skip 模式。**原判 refuted，本表採用改判計數（附錄 A）**：該檔共 **24** 支測試（不是 23），其中 **17 支**會呼叫 `Sim::spawn`（18 個呼叫點，其中兩個落在同一支），**7 支完全不碰模擬器**（`the_host_parses_the_locked_pair_fail_and_the_pairing_locked_hello`、`the_firmware_implements_the_same_pairing_lockout`、`the_firmware_advertises_its_name_in_the_scan_response`、`top_level_key_scanner_ignores_nested_objects`、`the_readme_command_parameter_ranges_match_the_hard_limits`、`the_firmware_gives_ble_notifications_a_length_discipline`、`the_firmware_ignores_aip_frag_and_never_claims_it_can_reassemble`）。**因此「整檔靜默變綠」是誇大**：無 python3 時至少 7 支仍跑完整斷言，另有 3 支在觸及模擬器前先跑韌體／README 對照。風險本身成立，規模不成立 |
| **I-17** | `scripts/drills/remove-package-keep-user-data.sh`（整支） | **沒有任何 runner 呼叫它**：`architecture-checks.sh` 的 `--drills` 只跑 `character-package.mjs`／`restricted-device.py`／`provider-disable-reenable.sh`／`optional-state.mjs`（本輪 stdout 也自報「四個 runner 實跑」）；`--drill-lint` 只做 `bash -n`（本輪 stdout：「6 支腳本靜態檢查通過（**未實跑**）」）。全 repo `grep -rn remove-package-keep-user-data` 在腳本／workflow 內**只命中它自己的用法註解**。它卻仍被 `docs/acceptance-evidence.md:1063`、`docs/releases/v0.7.0-migration.md:21`、`docs/releases/v0.7.0-final-report.md:73` 當成現行驗收證據。更明確的是：repo 自己的 v0.7.0 收斂證據 `docs/releases/evidence/2026-09-06-convergence/final/review-closeout/n5-final-drill-summary.md:140` 已寫明它只受 syntax lint、已被 `character-package.mjs` 取代——但那三份文件沒有跟著更新。**這是已經發生的文件與證據不一致，不只是腐爛風險** |

### 3.4 重複的昂貴 E2E、固定 sleep 與排序矛盾

| id | file:line | 為什麼低價值 |
|---|---|---|
| **I-12** | `apps/interaction-desktop/e2e/app.spec.ts:324` | 「緊急停止：二段確認觸發 → 誠實顯示 → 安全解除流程」與 `e2e/estop.spec.ts:37`／`:117`／`:192` 三案重疊；`estop.spec.ts:101-113` 的 `clearThroughSafetyFlow()` helper 就是同一串按鈕序列，而 `:117` 那支還多驗真後端效果（session 被取消、感測停止、手機收到 stop-all）。app.spec 版本唯一多出來的是「導覽後 `.topbar-title` 變成『連接與權限』」一句。**更嚴重的是排序矛盾**：`playwright.config.ts` 自己的註解說 estop 會撤銷同意、取消工作、停掉感測，所以 estop.spec 排在最後的 `estop-last` project；但 app.spec 屬於**最先跑的 `first-run`**，等於在所有 `main` 測試之前就觸發並解除過一次 estop |
| **I-13** | `apps/interaction-desktop/e2e/offline.spec.ts:6-13`（整檔 12 行 1 案） | 指向死埠 `127.0.0.1:19999` 斷言「系統無法啟動」；`e2e/evidence.spec.ts:706` 指向 `127.0.0.1:1` 走同一條路徑、斷言同一句文字，還多兩張截圖。差別只有 offline.spec 多一句「離線時不顯示權限地圖」的否定斷言，可直接併入。單 worker 序列下多一個 spec 檔就多一次 `goto` ＋ 一份 20 秒 timeout 預算（本輪實測 1.6 s） |
| **I-14** | `apps/interaction-desktop/e2e/app.spec.ts:311` | `await page.waitForTimeout(500); // let stored prefs land in the controlled input`——固定睡眠等後端偏好往返，之後才依 `yamlNav.count()` 分支。慢 runner 不夠、快機器浪費，失敗時分不清是逾時還是行為錯。**對照組**：`evidence.spec.ts` 裡 120–750 ms 的等待是截圖前的畫面穩定等待，屬合理用途，**不列為缺陷** |
| **I-18** | `apps/interaction-desktop/e2e/evidence.spec.ts`（15 案、751 行、13 次 `screenshot(`、107 個 `expect(`） | **原判 refuted，本表以改判為準（附錄 A）**。原判說它是 e2e job 最大的時間項——**本輪 `browser` 實跑證明不是**：`general-mode-tasks.spec.ts` 77.4 s（20 案）＞ `evidence.spec.ts` 45.4 s（15 案），資料來源 `baseline/pw-e2e.stdout.txt`，重算腳本見 §0.3。**仍然成立的理由**是另一個：它把「產出 `docs/assets` 文件截圖」與「功能回歸」混在同一支 spec，於是文件用途的截圖矩陣阻擋每一個 PR。它不是「純截圖」（107 個 `expect`），所以不能整支搬走 |
| **I-16** | `scripts/tests/ios-simulator.sh:7`、`:51` | `TASK_EXPECTED_COUNT=163` 寫死，傳給 `scripts/tests/xctest-result.py --expected-count`；`xctest-result.py:41` 的判定是 `total == expected` **嚴格相等**（`:56` 的預設值也是 163）。防靜默 skip 的意圖正確，但把「測試數量」變成契約：新增任何一支 XCTest 都會讓 runner 失敗直到有人改常數，方向雖安全，卻鼓勵「順手改大」而不是棘輪 |

### 3.5 中等信心／需要人決定（不列為缺陷）

| 候選 | 觀察 | 立場 |
|---|---|---|
| 17 個 `regressions-*` 檔（宣告 399 支，約占前端宣告數四分之一） | 每檔檔頭明列對應的對抗審查 finding id，品質高、**不是**低價值；但依審查輪次分檔，使同一行為的測試散在多個檔（例如「一般模式技術詞」同時出現在 `general-mode-no-technical-terms`、`regressions-v06-general-mode`、`regressions-v06-round2-general-mode`） | **keep**；至多做重新歸檔（依行為／頁面），**不刪任何案例**。歸檔是一次大 diff，收益與風險需人決定 |
| 角色 rig／遊玩場純函式測試（`src/test/rig.test.ts` 38 支等） | 執行成本近乎零。`rig.test.ts` 內對 36 個官方表情的長度斷言鎖的是產品規格數字，不是內部細節 | **keep** |
| `e2e/character-session.spec.ts`（4 案、本輪 22.6 s，是 E2E 第三大時間項） | 本輪才由實測資料浮現：4 個案例吃掉 22.6 s。dim-I 未討論 | `needs-investigation`：先量出時間花在哪（daemon 往返或固定等待），再決定要不要處置 |

---

## 4. 精簡處置表（**本階段只記錄，不執行**）

**規則**：
1. 這張表是**提案**。階段 0 不刪除、不合併、不改寫任何測試；執行與否由後續階段依 `phase-0-roadmap.md` 決定。
2. 「保護責任是否減少」問的是：照這個處置做完之後，**原本擋得住的缺陷是不是有一個沒人擋了**。
3. 標「**不得只因耗時而刪**」的列屬於安全、資料遷移或舊版相容回歸——即使它慢、即使它煩，**耗時不是刪除理由**。

| id | 標的 | 處置 | 替代案例（處置後由誰接手） | 保護責任是否減少 | 理由 | 分類 |
|---|---|---|---|---|---|---|
| **I-01** | `src/test/characterPage.test.tsx:778-786` | **改寫** | 同檔的 class 存在性與「> 390px 不得有 inline style」保留；版面行為交給 `e2e/narrow.spec.ts` 在 390px 量 `boundingBox()`／`getComputedStyle()` | 否——改寫後才第一次真的量到版面 | jsdom 不跑 layout，CSS 文字比對證明不了版面 | UI |
| **I-02** | `src/test/characterPage-first-screen.test.tsx:639-654` | **改寫** | 同 I-01；「不得有 transition／animation」改在 browser 驗 `getComputedStyle().transitionDuration` | 否 | 用中文註解當書籤，改註解就靜默切錯區段 | UI |
| **I-03** | `src/test/firstSuccess.test.tsx:196`（僅該行） | **刪除**（只刪這一行 CSS 文字斷言） | 同支測試的 class 存在性與「四個選項」數量斷言保留；窄視窗版面由 `e2e/narrow.spec.ts` 負責 | 否——padding 數值本來就不是這支測試的意圖 | 具體像素值與測試名脫鉤，改設計就誤紅 | UI |
| **I-04** | `src/test/regressions-v05-round2.test.tsx:70-79` | **改寫**（同層改行為測試） | 用 `configureHttp` ＋ `vi.stubGlobal("fetch")`：斷言第一個請求打 `/v1/status`、第二個請求的 `headers["Last-Event-ID"]`，再模擬 daemon 重啟斷言游標重置。同檔 `:38-68` 的 `nextStreamCursor` 純函式案例保留 | **是（暫時增加）**——改寫**之前**接線是零行為覆蓋，`api_e2e.rs::sse_stream_replays_with_last_event_id` 只涵蓋伺服器端 | 現況只鎖字串，真正的接線缺陷（忽略 `reset`）抓不到 | 誠實階梯相關（事件不得吞／不得重播過時綠勾） |
| **I-05** | `src/test/general-mode-metrics.test.tsx:219-246` | **改寫**（＋先抽函式） | 把分類判定從 `e2e/general-mode-tasks.spec.ts:1196-1216` 的 if/else 字面值抽成 `e2e/taskMetrics.ts` 的具名函式，改用一般 unit 測那個函式；`e2e/general-mode-tasks.spec.ts` 的「任務 11」仍驗後端真的 `cancelled` | 否 | 測試在測另一支測試的原始碼；條件寫錯它不會紅 | 測試基礎設施 |
| **I-06** | `src/test/deep-links.test.tsx:245-249` | **改寫** | `src/test/sensor-stop-history.test.tsx:12-18` 已示範 `vi.mock("../desktop")` 提供 `onNavigate` mock；改成觸發 mock callback 後斷言 topbar 標題換頁。同檔「Rust emit 的 tab 必須可路由」「⌘K PAGES 表全部可路由」是有價值的盤點式檢查，**保留** | 否 | **原判 exact-string 脆弱 → 改判「部分成立」**：脆弱性成立，但「六處等價寫法」的舉證不成立（詳附錄 A） | UI 路由 |
| **I-07** | `src/test/companion-gateway-wiring.test.ts:263-283` | **改寫並升層** | 新增 contract：`crates/interaction-api/tests/api_e2e.rs` 對 `/v1/character/hello` 送 `reducedMotion:true` 斷言協商降級（目前該檔對 `reducedMotion` grep 零命中）；前端保留 spy 斷言參數。同檔上半（`h.limits`／`h.locale`／`inputEventFor` 對照／file-drop 不含路徑）是紮實行為測試，**保留** | **是（暫時增加）**——升層完成前不要動現有字串檢查 | 三層字串比對：任一層把值丟掉不會紅 | 誠實階梯相關（reduced 演出不得記成 exact） |
| **I-08** | `src/test/companion-presets.test.ts:53-58` | **合併** | 併進同檔 `describeCompanionState` 的逐句硬字串斷言，或改成「三檔位 label 互異且不等於 preset id」 | 否——現況只守「非空」 | 任何 5 字元字串都會過，擋不住它宣稱要擋的缺陷 | UI 文案 |
| **I-09** | `src/test/companion-presets.test.ts:76-79` | **改寫**（或刪除） | 改成來源盤點式檢查（grep `src/` 找第二個寫 `companionExpressiveness` 的入口）；同檔 `:81` 之後的行為測試已涵蓋正向路徑 | 否——但改寫比刪除好：原意（單一套用入口）仍有價值 | 只擋一個已刪識別字，改名即繞過 | 架構紀律 |
| **I-10** | `crates/interaction-runtime/tests/declarative_session_loop.rs:66-70`（23 處 `else { return; }`） | **改寫**（`return None` → `panic!`） | **無替代**——這是裝置生命週期唯一的自動化證據 | **否**，且改寫後**顯著增加**：從「可能 0 個斷言」變成「要嘛真跑、要嘛硬失敗」 | 靜默變綠讓 `architecture-checks.sh:55` 的 `adapter-lifecycle` 四條關卡在最需要時變裝飾 | **安全／裝置生命週期：不得只因耗時而刪** |
| **I-11** | `crates/interaction-adapter-declarative/tests/esp32_sim_conformance.rs:66-70`（17 支受影響） | **改寫**（同 I-10） | 無替代（`real-hardware` 為 0 筆） | 否，改寫後增加 | 同 I-10；**規模以改判為準**：24 支中 17 支受影響，7 支不受影響 | **裝置線協定一致性：不得只因耗時而刪** |
| **I-12** | `e2e/app.spec.ts:324`（estop 那一案） | **合併**（併入 `e2e/estop.spec.ts`） | `estop.spec.ts:37`／`:117`／`:192` 三案（其中 `:117` 另驗真後端效果）；`src/test/dialog.test.tsx` 的確認鈕；`api_e2e.rs` 的 estop 路由案例；`crates/interaction-cli/tests/cli_e2e.rs` 的 estop 退出碼 | 否——併入後覆蓋更多，且順手解掉 `first-run` 先觸發 estop 的排序矛盾。**唯一要保留的差異**是「導覽後 topbar 標題」那一句，需一併搬過去 | 單 worker 序列下是純粹的重複成本 ＋ 與 config 註解的排序意圖直接矛盾 | 安全流程（**合併不是刪除**；estop 覆蓋總量不得下降） |
| **I-13** | `e2e/offline.spec.ts`（整檔） | **合併**（併入 `e2e/evidence.spec.ts:706`） | 只有 `evidence.spec.ts:706`——`grep -rln 系統無法啟動 src/ e2e/` 只命中 `src/App.tsx`、`e2e/evidence.spec.ts`、`e2e/offline.spec.ts` 三處，**沒有任何 vitest 案例**守這個畫面 | 否——**前提是**把 offline.spec 的否定斷言（離線時不顯示權限地圖）一起搬過去；漏搬就是減少 | 同一條路徑、同一句斷言，多一個 spec 檔多一次 `goto` | 誠實顯示（離線不得假裝正常） |
| **I-14** | `e2e/app.spec.ts:311` | **改寫** | 改成 `expect.poll(() => toggle.isChecked())` 或 `toBeEnabled()` | 否 | 固定 sleep 是 flake 來源，且失敗訊息無法區分逾時與行為錯 | 測試穩定性 |
| **I-15** | `e2e/helpers.ts:56-58` | **改寫** | label 從 `public/characters/index.json` 的 default manifest 讀（helpers 已有 `repoRoot()`）、marker 改 `data-testid`；`e2e/character.spec.ts` 與 `src/test/characterName.test.tsx`（23 支）繼續守角色名解析 | 否 | 換預設角色會讓約 10 支 spec 一起紅，且訊息指向導覽而非真因 | 測試基礎設施 |
| **I-16** | `scripts/tests/ios-simulator.sh:7`、`xctest-result.py:41/:56` | **改寫** | 改成「下限（`-ge`）＋版本化的棘輪值」，保留「數量下降就失敗」這個防靜默 skip 的核心 | 否——只要棘輪保留 | 嚴格相等把數量變成契約，鼓勵順手改大 | 防靜默 skip：**機制不得移除，只調整比較方式** |
| **I-17** | `scripts/drills/remove-package-keep-user-data.sh` | **保留 ＋ 接上 runner** | 接進 `architecture-checks.sh --drills`；在接上之前，`docs/acceptance-evidence.md:1063`、`docs/releases/v0.7.0-migration.md:21`、`docs/releases/v0.7.0-final-report.md:73` **不得**把它列為現行驗收證據（應改標為「只受 syntax lint」）。`src-tauri/src/character_store.rs`（15 支）覆蓋 store 層，但**不**覆蓋「移除套件後使用者偏好檔位元不變」 | **是（現在就已經減少了）**——它現在提供 0 保護，卻被三份文件當成證據 | 沒有 runner 呼叫；repo 自己的收斂證據已記錄此事，文件未同步 | **舊版相容／資料保留：不得只因耗時而刪**；文件對帳需立即修正 |
| **I-18** | `e2e/evidence.spec.ts` | **降層／拆 job** | 把文件截圖矩陣拆成獨立或 release-only job；把其中真正的回歸斷言（載入失敗降級、傳輸錯誤、離線）搬回 `app.spec.ts`／`home-state.spec.ts`。**同時**：若目標是縮短 e2e job，真正的最大時間項是 `general-mode-tasks.spec.ts`（77.4 s vs 45.4 s，本輪實測），應優先量測它 | **是（若截圖搬走時漏搬回歸斷言）**——因此拆分必須逐案標記「這是截圖」或「這是回歸」 | 混用文件產出與功能回歸；**「最大時間項」的原判已被本輪實測推翻** | 文件證據產出 ＋ 誠實顯示回歸（回歸斷言不得隨截圖一起下架） |

### 4.1 不得只因耗時而刪的清單（彙總）

以下即使慢、即使覆蓋看起來重疊，**耗時不構成刪除理由**：

- **安全**：`crates/interaction-policy/src/lib.rs` 全部 10 支；`crates/interaction-api/tests/api_e2e.rs` 的四支 human-only／agent-token 案例；`crates/interaction-runtime/tests/consent_one_shot_loop.rs`（10 支）；`estop_parallel.rs`（4 支）；`crates/interaction-agent-gateway/src/process.rs` 的 4 支 kill-tree 案例。
- **資料遷移**：`character_session_loop.rs` 的四支 snapshot 遷移／保留／隔離案例；`crates/interaction-character/tests/migration_registry.rs`（10 支）。
- **舊版相容**：`src/test/semantic-state-contract.test.ts` 的 `releases/v0.7.0/` 凍結 corpus（`sourceSha` 鎖定）；`crates/interaction-aip/tests` 的全部向量；`providers_loop.rs::a_legacy_disabled_provider_without_the_off_marker_stays_locked_after_restart`。
- **誠實階梯**：`agents_loop.rs` 與 `gateway_loop.rs` 的全部狀態機案例。
- **裝置線協定**：I-10 與 I-11 兩檔——它們是 `real-hardware` 為 0 筆的情況下**唯一**的協定證據，必須先修好靜默 skip，不能因為「要 python3、很慢」而放生。

---

## 5. 階段 0 新增的基線 runner 在矩陣中的位置

`scripts/tests/phase0/`（**本階段新增，尚未 commit**）是目前唯一走真 Agent 正式路徑的 harness。
它只走 `interact-ai serve` 真 daemon ＋ HTTP API ＋ SSE，不注入狀態、不偽造完成事件；每一步保留精確命令、
來源 commit、起訖時間、exit code 與 stdout／stderr。腳本用法見 `scripts/tests/phase0/README.md`。

| 腳本 | 在矩陣裡負責哪些格子 | 證據層級 | 執行觸發（現況） |
|---|---|---|---|
| `baseline.sh` | **不是新的測試**，是既有測試的統一紀錄器：20 步涵蓋 §1.6 表格的四個 CI job ＋ CI 沒有的 `architecture-checks` 六組、`v03-cli-e2e.sh`、`ios-simulator.sh`、`firmware/compile.sh` | `unit`／`contract`／`fixture`／`simulator`／`integration`／`browser` | **人工**：`PHASE0_OUT=<dir> scripts/tests/phase0/baseline.sh [step…]`。不在 CI、不在發布關卡 |
| `agent_smoke.py` | §2.1「誠實階梯」「取消要殺整棵程序樹」的真 agent 端；§2.2「memory TTL／刪除」的 provider transcript 面 | `real-agent` | **人工**；本輪用它發現 D1（claude interrupt → `failed` 而非 `cancelled`，5/5 重現） |
| `multi_session.py` | §2.1「session 隔離」——**矩陣裡唯一「自動化零覆蓋」那一格的現有唯一 harness** | `real-agent` | **人工**；本輪 E0-06／E0-07 `completed`（claude×2、codex×2、claude＋codex） |
| `restart_test.py` | §2.1「重啟後未完成工作是 unknown」的**硬殺**路徑（既有自動化只有優雅重啟） | `real-agent` | **人工**；本輪跑 `--signal KILL`，`--signal TERM` **未跑**（`analysis/e2e-baseline.json` E0-04 標 `harness-failed`，不是環境缺口） |
| `run-step.sh` | 單步紀錄器：`PHASE0_OUT=<dir> run-step.sh <name> <workdir> <cmd…>` → 追加一行到 `summary.jsonl` | — | 由上面各支呼叫 |
| `archive-evidence.py` | 把 scratchpad 原始證據歸檔到 `docs/releases/evidence/<date>-phase-0/`，略過 `home/`、掃描憑證、產 `artifact-manifest.json` | — | 人工 |

### 5.1 建議的觸發條件（提案，本階段不實作）

| 對象 | 建議觸發 | 理由 |
|---|---|---|
| `architecture-checks.sh --docs`／`--swift`／`--drills` | 接進 CI（`docs` 與 `swift` 每 PR；`drills` 每日或 release 前） | `rust`／`ts` 兩組已是 CI 子集；後三組是 CI 完全沒有的，而 `--swift` 正是「三端一致」缺的第三端的**便宜近似** |
| `scripts/v03-cli-e2e.sh`（96 斷言） | 接進 CI 或至少接進 `release-verify.sh --run-tests` | 它是 CLI 這個正式入口的唯一驗收；目前 CLI 端連「agent token 不能授予同意」都沒測（§2.1 表 B） |
| `ios-simulator.sh`（163 支） | macOS runner，每 PR 或每日 | 沒有它，`receive` 決策表與 canonical hash 的「三端一致」在 PR 上只有兩端 |
| `phase0/multi_session.py` | 先補一支**fixture 版**的 session 隔離自動化測試進 CI `rust`；真 agent 版維持人工、低頻 | 隔離是 runtime 責任，fixture 就能證明；真 agent 版昂貴且不確定，適合當加強證據而非 gate |
| `phase0/agent_smoke.py`、`restart_test.py` | 維持人工、低頻（發布前 ＋ 連接器改動時） | 真 agent 花錢、慢、受額度影響（本輪觀察到 claude 冷啟動一律約 61–65 s，原因未證實），不適合當每 PR gate |
| 4 支 Tauri AX 走查 | 維持人工，但**先修 harness**：合成鍵盤事件掉字與 `choosefile` 不驗證選檔（`analysis/e2e-baseline.json` 的 environmentBlockers） | 在 harness 修好之前，它們的綠燈不足以當證據——本輪 E0-11 已懷疑 2026-09-06 的綠燈可能是假陽性 |

---

## 附錄 A：懷疑者改判紀錄（dim-I，18 條 claim 中 3 條 refuted）

`docs/releases/evidence/2026-09-07-phase-0/static-matrix/dim-I.json` 的 `verdicts[]` 共 18 列，
`refuted:true` 者 3 列。**本文件正文一律以 `correctedStatus` 為準**：

| id | 原判 → 改判 | 理由摘要 | 本文件正文採用的版本 |
|---|---|---|---|
| **I-06** | 「App.tsx 另有六處等價寫法，換成任一種即誤紅」 → **部分成立：exact-string 確實脆弱，但「六處等價寫法」的舉證錯誤** | `App.tsx:257` 是 `useEffect` unlisten 陣列裡對 `./desktop` 模組函式的一次呼叫；`:344/453/513/525/539` 是 JSX 屬性語法、傳給五個子元件各自的 `onNavigate` prop，只是同名，語法上不可代換。**成立的窄化理由**：`:513`／`:539` 本身就用 `onNavigate={goTo}` 風格，把 `:257` 簡化成 `onNavigate(goTo)` 是合理重構，卻會讓 exact-match 誤紅 | §3.1 I-06、§4 I-06 均只寫窄化後的理由，並註明原判與改判 |
| **I-11** | 「23 個測試，整檔靜默變綠」 → **改正計數：全檔 24 支，其中 17 支呼叫 `Sim::spawn`；7 支完全不碰模擬器，另 3 支在觸及模擬器前先跑真實斷言** | 「整檔靜默變綠」誇大了影響範圍：無 python3 時仍有至少 7 支跑完整斷言。底層風險（一批協定一致性檢查靜默降為零斷言卻報 ok）成立 | §3.3 I-11、§4 I-11 採用 24／17／7 這組數字。**本文件作者以獨立腳本重新確認**：24 支測試、18 個 `Sim::spawn` 呼叫點分佈在 17 支測試、7 支不含任何呼叫（腳本見 §0.3 精神，逐支列名見 §3.3） |
| **I-18** | 「`evidence.spec.ts` 是 e2e job 最大的時間項」 → **每檔事實成立，但「最大時間項」不成立** | 懷疑者以靜態代理量（行數、案例數、`test.setTimeout` 總和）指出 `general-mode-tasks.spec.ts` 更大。**本文件進一步用本輪 `browser` 實跑資料證實**：`general-mode-tasks.spec.ts` 77.4 s／20 案 ＞ `evidence.spec.ts` 45.4 s／15 案（`baseline/pw-e2e.stdout.txt`） | §3.4 I-18、§4 I-18 改以「混用文件截圖與功能回歸」為處置理由，並在處置欄註明真正的最大時間項 |

其餘 15 條 verdict 為 `refuted:false`（`correctedStatus` 多為 `holds`）；其中 I-09 與 I-17 的懷疑者另加了兩點修正，
本文件已吸收進正文：

- **I-09**：「從此不可能失敗」是修辭上的誇大——它仍會攔截「原樣重新引入 `applyCompanionPreset` 這個識別字」。
  準確說法是「**任何改名都能繞過**」。§3.2 已改用後者。
- **I-17**：不只是腐爛風險，而是**已經發生的不一致**——repo 自己的
  `docs/releases/evidence/2026-09-06-convergence/final/review-closeout/n5-final-drill-summary.md:140`
  已寫明該腳本只受 syntax lint、已被 `character-package.mjs` 取代，但三份現行文件仍引用它。§3.3／§4 已採用。

## 附錄 B：completeness critic 與未解問題

**completeness critic**：`static-all.json` 的 12 個維度中，A／B／C／D／E／F／G／K／L 九個維度有 `critic` 區塊
（含 `missingRows`），**H／I／J 三個維度沒有**。因此**維度 I 沒有 completeness critic 提出的 missingRows**，
本附錄無此類條目可列。（若後續補跑 critic，其產出必須標「completeness critic 提出，未逐列核實」才可引用。）

**dim-I 自列的未解問題**（原樣轉錄，並標註本文件的處理）：

| dim-I 的問題 | 本文件的處理 |
|---|---|
| session 隔離在 repo 內找不到任何自動化測試——是刻意接受的風險還是漏掉的？ | §2.1 表 B 標 `absent`（自動化）／`implemented-partial`（`real-agent`）。**仍是未解問題**，需人決定 |
| 備份還原沒有後端 import／restore 路由，「備份與還原」這個 IA 名稱是否已超出實作範圍？ | §2.2 表 B 標 `needs-investigation` 並列出 `crates/interaction-api/src/lib.rs:193` 只有 export。**仍是未解問題** |
| `GatewayEvent::TaskWaitingForInput` 沒有 connector 產生——是預留介面還是應該有真 agent 證據？ | §2.1 表 B 標 `defined-only`（grep 已確認只有定義處與消費端）。**仍是未解問題** |
| `architecture-checks.sh` 的 docs／swift／drills 三組要接進 CI，還是明確接受只在 release 前手跑？ | §5.1 提出接進 CI 的建議。**待人決定** |
| iOS XCTest 163 支不在任何 CI job，要不要在 macOS runner 上跑？ | §5.1 提出建議。**待人決定** |
| 17 個 `regressions-*` 檔重新歸檔的收益是否值得一次大搬動的 diff 風險？ | §3.5 立場：內容全部保留，只考慮歸檔。**待人決定** |
| dim-I 本輪未執行任何測試，靜態計數需與實跑基線對帳 | **已對帳**：§1 每一節都並列宣告數與本輪實跑數，並修正了 dim-I 的前端檔數（94→92）與 `regressions-*` 檔數（13→17） |
| repo 內完全沒有 axe／jest-axe 的引用，是否要在 `a11y.spec.ts` 加 axe 掃描？ | §2.4 表 B 已確認 grep 零命中並標為缺口；§2.4 表 A 建議「靜態規則用 axe 對五入口各跑一次」。**待人決定** |

---

## 附錄 C：本文件引用的證據檔（全部為 repo 相對路徑）

| 用途 | 路徑 |
|---|---|
| 靜態盤點與懷疑者 verdicts | `docs/releases/evidence/2026-09-07-phase-0/static-matrix/dim-I.md`、`dim-I.json`、`static-all.json` |
| 本輪基線實跑（20 步的 cmd／exit／stdout／stderr） | `docs/releases/evidence/2026-09-07-phase-0/baseline/`（`summary.jsonl` ＋ 各步 `*.stdout.txt`／`*.stderr.txt`） |
| 真 Agent E2E 分類與缺陷清單 | `docs/releases/evidence/2026-09-07-phase-0/analysis/e2e-baseline.json` |
| 真 Agent E2E 原始資料（上一輪） | `docs/releases/evidence/2026-09-07-phase-0/real-agent-e2e-prior/`（含 `multi-C-claude-x2/`、`multi-D-codex-x2/`、`multi-E-claude-codex/`、`restart-claude-kill/`） |
| 本輪各回合原始資料 | `docs/releases/evidence/2026-09-07-phase-0/e2e-runs/R1`…`R8` |
| 本階段 harness 快照 | `docs/releases/evidence/2026-09-07-phase-0/harness-used/`（`agent_smoke.py`、`multi_session.py`、`restart_test.py`） |
| 歸檔清單 | `docs/releases/evidence/2026-09-07-phase-0/artifact-manifest.json` |
| v0.7.0 收斂期對 I-17 的既有記錄 | `docs/releases/evidence/2026-09-06-convergence/final/review-closeout/n5-final-drill-summary.md` |
