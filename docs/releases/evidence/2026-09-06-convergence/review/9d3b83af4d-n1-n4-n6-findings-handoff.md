# N1 / N4 / N6 findings handoff

Repository: `/Users/user/Workspace/claude-lab/adaptive-interaction`. Review base `622c8bf70963e343e51f92f5ecdf7fce4b37f516` plus shared uncommitted changes. No commit/push by this agent.

Findings handled by semantic_contract; does not replace parent full N1-N6 evidence index.

## Role and evidence record

### N1-CONTRACT-01 — State receive accepted missing required fields and destructive required-field merge

**P2; Existing implementation accepted states invalid under the canonical contract.**

Trigger: Remove attention, members or reducedMotion from a hash-consistent snapshot, or delete required members with merge-patch null. Explicit state null is also invalid.

Before: Receiver returned apply and adopted an invalid state.

Fixed behavior: Validate complete state before atomic adoption; merge-patch null retains deletion semantics and the merged result must satisfy the state contract.

Roles: finder=semantic_contract; red_reproducer=semantic_contract; author=semantic_contract; independent_verify=No separate reproduction of the original five cases is claimed here; integration corpus/gates are parent-owned..

Relative sources: `apps/interaction-desktop/src/aip/sessionClient.ts`, `apps/interaction-desktop/src/aip/semanticState.ts`, `crates/interaction-session/src/semantic_contract.rs`.

Relative tests: `apps/interaction-desktop/src/test/semantic-state-contract.test.ts`, `crates/interaction-session/tests/semantic_contract.rs`.

Red:
- `/tmp/semantic-contract-red.log` — passed=0, failed=5

Green:
- `/tmp/semantic-review-fix-ts.log` — passed=59, failed=0
- `/tmp/semantic-review-fix-rust.log` — passed=5, failed=0
- `/tmp/semantic-review-fix-swift.log` — passed=58, failed=0

Status: fixed; scoped contract, production receive and native Swift model tests pass.

### N1-RESTORE-02 — Authoritative snapshot restore silently discarded explicit null before validation

**P2; Existing invalid-input acceptance; this round adds enforcement, not a claim that the old guard covered it.**

Trigger: Restore a raw state snapshot with explicit null and a matching canonical hash. Serde Option accepts null and subsequent serialization omits it.

Before: Restore accepted the snapshot while changing canonical data.

Fixed behavior: Validate the raw state before deserializing, reject null atomically, retain authoritative deny-unknown policy.

Roles: finder=semantic_contract; contract_review=root; red_reproducer=semantic_contract; author=semantic_contract; independent_verify=Parent integration gate; no separate original-null probe log is claimed..

Relative sources: `crates/interaction-session/src/session.rs`, `crates/interaction-session/src/semantic_contract.rs`.

Relative tests: `crates/interaction-session/tests/semantic_contract.rs`.

Red:
- `/tmp/semantic-host-null-red.log` — passed=0, failed=1

Green:
- `/tmp/semantic-review-fix-rust.log` — passed=5, failed=0

Status: fixed; production restore regression passes.

### N1-REVIEW-03 — Swift flattened Attention DTO rejected a valid unknown-variant field

**P2; Introduced by this round generated DTO validation; found during independent review.**

Trigger: attention.kind=future-gaze or none with unknown optional id=42.

Before: Rust/TS accepted both; Swift rejected both through a flattened known id string member.

Fixed behavior: Generated DTO reader validates only the selected known variant and preserves raw unknown fields for hashing.

Roles: finder=root; independent_red_reproducer=root; author=semantic_contract; independent_postfix_verifier=root.

Relative sources: `scripts/semantic-state-codegen.mjs`, `apps/interaction-ios/InteractionCompanion/Models/SemanticStateGenerated.swift`, `apps/interaction-desktop/src/aip/semanticStateGenerated.ts`.

Relative tests: `crates/interaction-aip/tests/fixtures/semantic-state-cases.json`, `scripts/tests/semantic-state-conformance.swift`.

Red:
- `/tmp/adaptive-convergence-20260906/n1-ts-verify.log` — description=Both attention probes accepted
- `/tmp/adaptive-convergence-20260906/n1-swift-verify.log` — description=Both attention probes incorrectly rejected

Green:
- `/tmp/adaptive-convergence-20260906/n1-rust-verify.log` — cases=5
- `/tmp/adaptive-convergence-20260906/n1-ts-verify-fixed.log` — cases=5
- `/tmp/adaptive-convergence-20260906/n1-swift-verify-fixed.log` — cases=5

Status: fixed; all three validators agree on the five independent attention/date probes.

### N1-REVIEW-04 — Consumer date validation normalized invalid dates and rejected valid leap seconds

**P2; Introduced/left uncovered by this round consumer validation; found during independent review.**

Trigger: 2026-02-30T00:00:00Z, 2026-09-06T24:00:00Z, and 2016-12-31T23:59:60Z.

Before: TS/Swift accepted the first two and rejected the third; authoritative chrono did the opposite.

Fixed behavior: Explicit component validation follows existing chrono structural RFC3339 behavior, without Date.parse normalization or leap-table inference; raw/hash inputs unchanged.

Roles: finder=root; independent_red_reproducer=root; author=semantic_contract; independent_postfix_verifier=root.

Relative sources: `apps/interaction-desktop/src/aip/semanticState.ts`, `apps/interaction-ios/InteractionCompanion/Models/SemanticStateValidation.swift`.

Relative tests: `crates/interaction-aip/tests/fixtures/semantic-state-cases.json`.

Red:
- `/tmp/adaptive-convergence-20260906/n1-rust-verify.log` — description=Authoritative expected false,false,true
- `/tmp/adaptive-convergence-20260906/n1-ts-verify.log` — description=Consumers true,true,false
- `/tmp/adaptive-convergence-20260906/n1-swift-verify.log` — description=Consumers true,true,false

Green:
- `/tmp/adaptive-convergence-20260906/n1-ts-verify-fixed.log` — cases=5
- `/tmp/adaptive-convergence-20260906/n1-swift-verify-fixed.log` — cases=5
- `/tmp/semantic-review-fix-ts.log` — passed=59, failed=0
- `/tmp/semantic-review-fix-swift.log` — passed=58, failed=0

Status: fixed; 18 additional shared state cases pin the reviewed edges.

### N4-REVIEW-01 — Import reported settings unchanged after prefs had already committed

**P2; Existing two-command outcome conflation.**

Trigger: prefsPatch resolves after saving companionName, then companionApplyPrefs rejects with a lost apply response.

Before: The saved preference changes, but onPatch=false causes the library to say no write succeeded and settings are unchanged.

Fixed behavior: Parent separates persisted/apply outcome handling and uses conservative import failure wording.

Roles: finder=semantic_contract; independent_red_verifier=root; author=root; independent_postfix_verifier=semantic_contract.

Relative sources: `apps/interaction-desktop/src/pages/CompanionPage.tsx`, `apps/interaction-desktop/src/pages/character/CharacterLibrary.tsx`.

Relative tests: `apps/interaction-desktop/src/test/characterPage.test.tsx`.

Red:
- `/tmp/n4-import-review-red.log` — passed=0, failed=1, skipped=24
- `/tmp/adaptive-convergence-20260906/n4-import-independent-red.log` — failed=1

Green:
- `/tmp/n4-review-final-independent-green.log` — passed=41, failed=0

Status: fixed; production page with modeled two-IPC boundary passes.

### N4-REVIEW-02 — Atomic preference replacement widened an existing private file mode

**P2; Existing temp-and-rename implementation defect, not newly introduced this round.**

Trigger: Create desktop.json with mode0600, call production save_prefs_at under the observed022 umask.

Before: The replacement file becomes0644.

Fixed behavior: Parent preserves the existing permissions and creates private temporary files.

Roles: finder=semantic_contract; independent_red_verifier=root; author=root; independent_postfix_verifier=semantic_contract.

Relative sources: `apps/interaction-desktop/src-tauri/src/supervisor.rs`.

Relative tests: `apps/interaction-desktop/src-tauri/src/settings_recovery_review.rs`.

Red:
- `/tmp/n4-prefs-writer-review-red.log` — passed=0, failed=2
- `/tmp/adaptive-convergence-20260906/n4-writer-independent-red.log` — failed=2

Green:
- `/tmp/n4-writer-review-final-independent-green.log` — passed=2, failed=0
- `/tmp/adaptive-convergence-20260906/n4-writer-green.log` — passed=2, failed=0

Status: fixed; production writer on isolated real filesystem passes.

### N4-REVIEW-03 — Failed temporary create removed a pre-existing file the writer did not own

**P2; This round new unconditional error cleanup.**

Trigger: In a fresh test subprocess precreate the expected PID/sequence temporary path, then invoke save_prefs_at.

Before: create_new rejects the collision and leaves desktop.json intact, but unconditional cleanup deletes the colliding file.

Fixed behavior: Cleanup runs only for the temporary file successfully created by this invocation.

Roles: finder=root raised suspicion; semantic_contract reproduced; independent_red_verifier=root; author=root; independent_postfix_verifier=semantic_contract.

Relative sources: `apps/interaction-desktop/src-tauri/src/supervisor.rs`.

Relative tests: `apps/interaction-desktop/src-tauri/src/settings_recovery_review.rs`.

Red:
- `/tmp/n4-prefs-writer-review-red.log` — passed=0, failed=2
- `/tmp/n4-prefs-collision-review-red.log` — passed=0, failed=1
- `/tmp/adaptive-convergence-20260906/n4-writer-independent-red.log` — failed=2

Green:
- `/tmp/n4-writer-review-final-independent-green.log` — passed=2, failed=0

Status: fixed; failed production create preserves both original and foreign temporary data.

### N4-REVIEW-04 — Nested malformed backup fields were coerced into persisted strings

**P2; Existing malformed-input acceptance missed by new top-level guards.**

Trigger: companionFamiliars id=42, name=42, name={}, or palette=[maid-classic].

Before: String(...) silently accepts all four and returns a valid settings patch, including the name [object Object].

Fixed behavior: Parent requires string id/name/palette before returning a patch; valid schema1 backups retain their behavior.

Roles: finder=semantic_contract; independent_red_verifier=root; author=root; independent_postfix_verifier=semantic_contract.

Relative sources: `apps/interaction-desktop/src/companion/settingsTransfer.ts`.

Relative tests: `apps/interaction-desktop/src/test/settings-import-review.test.ts`.

Red:
- `/tmp/n4-nested-backup-review-red.log` — passed=0, failed=4
- `/tmp/adaptive-convergence-20260906/n4-nested-independent-red.log` — failed=4

Green:
- `/tmp/n4-review-final-independent-green.log` — passed=41, failed=0

Status: fixed; four nested cases plus existing page/top-level cases pass.

### N6-REVIEW-01 — Initial Shift+Tab escaped a newly opened modal and lost its Escape handler

**P2; Existing focus trap only guarded first/last children, not initial container focus.**

Trigger: Open the emergency-stop recovery dialog via Enter; container initially has focus; press Shift+Tab before entering any child.

Before: Browser focus leaves the dialog. An independently added unit case identifies the focused node as the background opener.

Fixed behavior: Initial Tab/Shift+Tab target first/last child; an empty loading overlay prevents Tab escape. Existing close and focus restoration remain.

Roles: finder=restart_lifecycle; independent_red_verifier=semantic_contract (CodeGraph refutation attempt, independent unit plus real Browser); author=semantic_contract (parent explicitly assigned after independent Verify); postfix_verifier=semantic_contract scoped tests; parent owns independent full regression.

Relative sources: `apps/interaction-desktop/src/components/Dialog.tsx`.

Relative tests: `apps/interaction-desktop/e2e/estop.spec.ts`, `apps/interaction-desktop/src/test/dialog.test.tsx`.

Red:
- `/tmp/n6-dialog-browser-independent-red.log` — passed=0, failed=1
- `/tmp/n6-dialog-independent-unit-red.log` — passed=6, failed=1

Green:
- `/tmp/n6-dialog-browser-green.log` — passed=1, failed=0, seconds=5.9
- `/tmp/n6-dialog-shared-unit-green.log` — passed=71, failed=0

Status: fixed; Browser verifies Tab containment, Escape closes, opener focused/in viewport, emergencyStop still true.

## Scope boundaries

N1 original five receive regressions, authoritative null regression, reviewed Attention/date probes, numeric metadata/transport work, shared corpus, frozen corpus and N5 drill evidence are detailed in `/tmp/semantic-contract-handoff.md`. Final contract corpus has44states+4patches; Rust5 tests, TS59 tests, standalone native Swift models58 tests. These counts are scoped suites and overlap the findings above; do not add them as independent cases.

N4 final independent UI/import check:41/0 in `/tmp/n4-review-final-independent-green.log` (25page+12top-level+4nested). Native production writer:2/0 in `/tmp/n4-writer-review-final-independent-green.log`. These share tests across findings; do not count each finding row as a fresh suite.

N6 scoped Browser:1/0 in `/tmp/n6-dialog-browser-green.log`; shared unit suites:71/0 in `/tmp/n6-dialog-shared-unit-green.log`. Red trace retained in `/tmp/n6-dialog-independent-red-artifacts`. Browser uses a real isolated daemon and fixture setup; it does not manipulate native App state or stand in for human emergency unlock.

The native preset recovery script label is truthful. It does not verify every failure/retry sentence, skips the final UI preset assertion for newer-choice, and contains no backup export/import or downloaded-file assertion. Parent owns supplemental native backup evidence and full release gates.

JSON source of this handoff: `/tmp/n1-n4-n6-findings-handoff.json`.
