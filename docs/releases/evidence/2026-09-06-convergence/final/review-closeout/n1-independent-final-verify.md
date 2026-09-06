# N1 獨立原始 Finding Verify

Verifier: `/root/state_receipts`（非原作者）。原始基線 `622c8bf70963e343e51f92f5ecdf7fce4b37f516`；修復版 `3b375bde4188e4a039852881d48d82ea9b619ce1`。

兩項原始缺陷均由未修改的 Git production source 重現，並以同一組獨立 probe 在修復版回綠。不是逆補丁或 mutation。共享 repo tracked files、App、CLI 和 native UI 均未修改／重建／操作。

| Finding | 原版 red | 修復版 green | 證據範圍 |
|---|---|---|---|
| N1-CONTRACT-01 | 2 控制通過、5 無效狀態斷言失敗 | 7/0 | production TypeScript reduce 原子接收邊界 |
| N1-RESTORE-02 | 2 控制通過、1 null 斷言失敗 | 3/0 | production CharacterSession::restore / serde / canonical |

## 嘗試反駁與結果

- **Hash mismatch, wrong generation or wrong identity caused the reported rejections instead of a semantic contract defect.** Independent envelopes use session.home, arrival generation0, epoch1 and hashes calculated by each version own production canonical helper. A full valid snapshot is accepted in both trees. Refuted: baseline accepts all five invalid states; current rejects all five. The normal control remains accepted.

- **Merge-patch null is always prohibited, so deleting members does not distinguish whole-state validation.** Independent positive control deletes only an optional unknown extension through RFC7396 null; separate negative probe deletes required members and supplies the correct post-deletion hash. Refuted: optional deletion applies in both versions. Required deletion improperly advances baseline to revision2; current rejects and preserves identical prior local state at revision1.

- **The consumer is never reached with this input because a UI/transport upstream already enforces complete SemanticState.** Read baseline transport JSON.parse and CharacterSyncCard event dispatch into production reduce. The reducer is the actual application receive/adoption boundary and baseline has no SemanticState validator there. Refuted within the observed production receive path: CharacterSyncCard.tsx:219-220 dispatches reduce, :435 sends event.payload; transport.ts JSON parsing does not impose required SemanticState fields. Tests call that production boundary, not a mock.

- **Missing fields/null are intentional old-snapshot migration or allowed optional representation.** Read original 622c8bf docs/aip/character-session.md sections3 and6 and original SemanticState struct in state.rs:275-287. attention/members/reducedMotion are required, lastInteraction uses serde Option skip-if-none, and original contract says complete canonical state must contain no null. Refuted for these probes. Compatibility defaults for older optional member fields do not make required top-level fields optional or authorize explicit null. The original JSONC illustration uses null but its explicit implementation note at68 and invariant at83 require omission.

- **Authoritative restore hash/deny-unknown guards already prevent raw null adoption.** Production CharacterSession::restore called with an independently made snapshot containing lastInteraction:null and recomputed matching hash. Controls restore a valid snapshot and reject an unknown authoritative field. Refuted: both controls pass at baseline, yet explicit null is accepted and discarded, changing canonical hash. Fixed raw contains_null check rejects InvalidState before serde.

## 精確執行

**baseline / ts** cwd `/tmp/n1-independent-final-verify/baseline/apps/interaction-desktop`；exit 1；2 passed / 5 failed。Wall time 0.511s；編譯 見logs。

```sh
CARGO_TARGET_DIR=/tmp/n1-independent-final-verify/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 node /Users/user/Workspace/claude-lab/adaptive-interaction/apps/interaction-desktop/node_modules/vitest/vitest.mjs run --config vitest.independent.config.ts
```
[完整日誌](n1-original-verify/baseline-ts.txt)

**baseline / rust** cwd `/tmp/n1-independent-final-verify/baseline`；exit 101；2 passed / 1 failed。Wall time 10.86s；編譯 見logs。

```sh
CARGO_TARGET_DIR=/tmp/n1-independent-final-verify/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 cargo test --offline -p interaction-session --test independent_restore -- --nocapture
```
[完整日誌](n1-original-verify/baseline-rust.txt)

**current / ts** cwd `/tmp/n1-independent-final-verify/current/apps/interaction-desktop`；exit 0；7 passed / 0 failed。Wall time 0.478s；編譯 見logs。

```sh
CARGO_TARGET_DIR=/tmp/n1-independent-final-verify/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 node /Users/user/Workspace/claude-lab/adaptive-interaction/apps/interaction-desktop/node_modules/vitest/vitest.mjs run --config vitest.independent.config.ts
```
[完整日誌](n1-original-verify/current-ts.txt)

**current / rust** cwd `/tmp/n1-independent-final-verify/current`；exit 0；3 passed / 0 failed。未另量完整 wall time；編譯9.34s，3 tests執行0.00s（依log）。

```sh
CARGO_TARGET_DIR=/tmp/n1-independent-final-verify/target-current CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 cargo test --offline -p interaction-session --test independent_restore -- --nocapture
```
[完整日誌](n1-original-verify/current-rust-clean-target.txt)

## 原始來源與限制

- [JSON 完整報告](n1-independent-final-verify.json)、[來源 manifest](n1-original-verify/source-manifest.json)、[完整 production diff](n1-original-verify/production.diff)，附所有 production source SHA-256 與 probe／log hash。
- 隔離 root Cargo.toml 只將 workspace members 縮為 aip/character/session；production 檔逐一與 git show 比對完全相同。獨立 targets 合計約 339 MiB。
- 第一輪 current Rust 共用隔離 target 發生 Cargo historical-mtime stale artifact 問題，該失敗明確排除；以全新 target-current 重編 9.34s 後 3/0。原始 stale log 保留，replay script 已改兩個 target。
- 基線 restore 在 hash 一致下接受 lastInteraction:null，還原後刪掉欄位，輸入與輸出 hash 不同；current 回 InvalidState。正常 snapshot 及 unknown-authoritative-field 拒絕控制都在兩版通過。
- 這是 production 邊界及 pure crate 驗證，沒有冒稱 browser/native UI、Swift、filesystem adapter 或真機驗收。
