# Final Find / Verify delivery map

Audit is read-only. No tests, native UI, build or performance run was started by this auditor.

Audit HEAD: `3b375bde4188e4a039852881d48d82ea9b619ce1`. Source fixes: `30ff852`, `87b0d031`; the last native result predicate fix: `3b375bd`.

Unique rows including requested deltas: **30**. Product/harness defects still open: **0**. Independent verification still pending: **none**.

Roles below are implementation author / Finder / Verifier. N1=semantic_contract, N2=state_receipts, N3=restart_lifecycle; root is integrator. These are AI review roles, not human acceptance.

| ID (aliases) | Author / Finder / Verifier | Evidence and remaining scope | Fix | Open |
|---|---|---|---|---|
| N1-CONTRACT-01 | N1 / N1 / N2 | author pre-fix regression; author post-fix regression; independent original-source pre-fix reproduction; independent post-fix verification | `30ff852f` | no |
| N1-RESTORE-02 | N1 / N1 / N2 | author pre-fix regression; author post-fix regression; independent original-source pre-fix reproduction; independent post-fix verification | `30ff852f` | no |
| N1-REVIEW-03 | N1 / root / root | independent pre-fix reproduction; independent post-fix verification | `30ff852f` | no |
| N1-REVIEW-04 | N1 / root / root | independent pre-fix reproduction; independent post-fix verification | `30ff852f` | no |
| N4-REVIEW-01 | root / N1 / root | finder pre-fix reproduction; independent pre-fix reproduction; independent post-fix verification | `30ff852f` | no |
| N4-REVIEW-02 (N4-WRITER-01) | root / N1 / root | finder pre-fix reproduction; independent pre-fix reproduction; independent post-fix verification | `30ff852f` | no |
| N4-REVIEW-03 (N4-WRITER-02) | root / root raised suspicion; N1 reproduced / root | finder pre-fix reproduction; independent pre-fix reproduction; independent post-fix verification | `30ff852f` | no |
| N4-REVIEW-04 | root / N1 / root | finder pre-fix reproduction; independent pre-fix reproduction; independent post-fix verification | `30ff852f` | no |
| N6-REVIEW-01 | N1 (parent explicitly assigned after independent Verify) / N3 / N1 (CodeGraph refutation attempt, independent unit plus real Browser) | independent pre-fix reproduction; author post-fix regression; independent post-fix verification | `30ff852f` | no |
| N4-UI-01 | /root / N3 / N2 | independent pre-fix reproduction; independent post-fix verification | `30ff852f` | no |
| N4-UI-02 | /root / N3 / N2 | independent pre-fix reproduction; independent post-fix verification | `30ff852f` | no |
| N4-UI-03 | /root / N2 / /root | finder pre-fix reproduction; parent independent reasoning reported; independent post-fix verification | `30ff852f` | no |
| N3-SCOPE-01 | N3 / N1 / N2 | finder pre-fix reproduction; independent pre-fix reproduction; independent post-fix verification; author post-fix full scoped regression; independent latest post-fix verification | `30ff852f` | no |
| N3-SCOPE-02 | N3 / N1 / N2 | finder pre-fix reproduction; independent pre-fix reproduction; independent post-fix verification; author post-fix full scoped regression; independent latest post-fix verification | `30ff852f` | no |
| N3-SCOPE-03 | N3 / N3 / N2 | author pre-fix regression; independent pre-fix reproduction; independent post-fix verification; author post-fix full scoped regression; independent latest post-fix verification | `30ff852f` | no |
| N3-MOBILE-EDGE-01 (N3-MOBILE-01) | N3 / N2 / /root | finder pre-fix reproduction; independent pre-fix reproduction; author post-fix full scoped regression; independent post-fix verification | `30ff852f` | no |
| N3-RESTART | N3 / user N3.1 / author regression plus independent cross-scope review; no per-case original independent red claimed | author pre-fix delta regression; author post-fix scoped regression | `30ff852f` | no |
| N3-GENERATION | N3 / user N3.1; parent approved the generation evidence boundary / author regression plus independent cross-scope review; no per-case original independent red claimed | author pre-fix delta regression; author post-fix scoped regression | `30ff852f` | no |
| N3-SHUTDOWN | N3 / user N3 persistence requirement / author regression plus independent cross-scope review; no per-case original independent red claimed | author pre-fix delta regression; author post-fix scoped regression | `30ff852f` | no |
| N3-E2E-SENSOR-TEARDOWN | N3 / N3 / differential live-daemon Browser evidence; parent full-suite followup pending | finder differential control; finder pre-fix reproduction; author fixture post-fix regression; independent full-suite post-fix verification | `87b0d031` | no |
| N3-UI-STOP-HISTORY | N3 / N3 (candidate from failing Browser trace) / /root (independent App trace and recorded real Browser/network evidence before authorization) | author pre-fix regression; author post-fix regression; independent pre-fix isolated reversal; independent post-fix verification | `87b0d031` | no |
| SCRIPT-01 | N1 after explicit parent authorization / N1 / root | finder/author pre-fix parser regression; independent pre-fix parser reproduction; author post-fix regression; independent post-fix regression | `87b0d031` | no |
| SCRIPT-02 | root / N1 / N1 independent post-fix guard/source review | independent post-fix extracted production guard probes | `87b0d031` | no |
| SCRIPT-03 | root / N1 / N1 independent post-fix guard/source review | finder conditional reproduction and independent post-fix selector probes | `87b0d031` | no |
| SCRIPT-04 | root / N1 / N1 independent post-fix guard/source review | independent post-fix extracted production guard probes | `87b0d031` | no |
| DOC-01 | root / N1 / N1 independent post-fix guard/source review | independent post-fix static/source verification | `87b0d031` | no |
| DRIVER-01 | root / N1 / N1 independent post-fix guard/source review | independent post-fix static/source verification | `87b0d031` | no |
| native-result-gate-emergency-status | root / N1 / N1 before/after exact predicate probes | independent finder pre-fix exact guard probes; independent post-fix exact guard probes | `3b375bde` | no |
| ARCH-SWIFT-BOUNDARY | N2 / /root / root independent original and fixed runner | independent finder original runner failure; fix-author fixed runner regression; independent clean-checkpoint post-fix verification; independent aggregate runner post-fix verification | `87b0d031` | no |
| N2-E2E-LEGACY-01 | N2 / /root (integration run) / N2 | independent integration discovery; fix author scoped Browser regression; independent complete Browser verification | `87b0d031` | no |

## Five review dimensions

This maps the current change and affected boundaries. The five-agent workflow was not invoked, and the v0.7 whole-repository findings count is not imported.

| Dimension | Actual scope and evidence | Limits |
|---|---|---|
| safety-bypass | Preset configure cannot manufacture consent or invoke actuators; API agent write allowlist excludes config. Generated recipe plans still go through authorize_step and last-instant stop/consent gate. Mobile raw capture and source rebind remain conservative after disconnect/restart. changed-boundary source review plus independently executed scoped N3 port regressions; policy/executor exact baseline bytes unchanged | No new exhaustive token/transport attack audit; no physical actuator consent acceptance. |
| state-machine | Raw semantic adoption is atomic; state-applied token/generation/session/epoch/revision/hash/messageId binding, bounded outstanding and monotonic timeout, legacy unconfirmed status. Unknown-stop owner scopes and config CAS/operation receipt recovery are independently challenged. pure production model, actual controlled React component, pty/TLS production ports and parent tracker4/0; common ActionReceipt/executor code unchanged | stateApplied is a negotiated semantic application receipt, not physical execution proof; old ActionReceipt internals not newly exhaustively audited. |
| async-hygiene | Late/overlapping prefs reads and reconnect projection; bounded32 outstanding receipts/5s TTL. Sensor stop is bounded before transport shutdown; scope confirmation uses nonblocking try_read while connection is pinned. Config writer holds one lock with synchronous durable write and no new await/re-lock after acquiring it. independent controlled async component/production-port tests plus scoped lock/cancellation/bounds source inspection | SQLite/OS latency remains unbounded by transport deadline and can delay raw capture exposure. Busy try_read conservatively leaves a reminder. No fresh load, deadlock stress or performance run by auditor. |
| api-contract | Canonical raw semantic required fields, null, unknown numeric fields and date parity; host restore boundary; config conflict HTTP409 and idempotent operation receipt; backup type/commit outcome honesty; unresolved summary/health in three UI stop call paths; legacy phone exact applied diagnostics. independent original/fixed pure production boundary, native Swift model, controlled actual React, Browser + production daemon fixture | No new full canonical-tool-manifest dispatch inventory audit; changed stop/config/status routes and projections are the reviewed scope. |
| recipe-engine | Proactive configuration only changes config/revision/last operation, preserves cooldown/usage/quiet/dedup fields, and rolls back full prior state on failed save. Presets write only mode plus revision/operation metadata. Recipe evaluation, trigger/fusion/condition, orchestrator and executor are byte-identical to622c8bf; generated completion still uses Autonomous execute_plan and Governor. current source inspection +9 exact git byte/section comparisons; no new recipe execution claimed | No fresh exhaustive ordering/window/fusion/scoring/cyclic-trigger algorithm audit. Those algorithms were unchanged; this checks the touched configuration-to-executor boundary only. |

No necessary changed-scope review gap was identified after the supplemental read-only recipe/configuration boundary inspection. This is not a full old-code correctness claim.

## Evidence details

### N1-CONTRACT-01 — State receive accepted missing required fields and destructive required-field merge

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: independently verified original defect; fixed behavior confirmed.

- author pre-fix regression — semantic_contract; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI; passed=0, failed=5. `docs/releases/evidence/2026-09-06-convergence/review/ceffee521e-semantic-contract-red.txt`
- author post-fix regression — semantic_contract; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI; passed=59, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/1c84d147ba-semantic-review-fix-ts.txt`
- author post-fix regression — semantic_contract; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI; passed=5, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/32dfb8961a-semantic-review-fix-rust.txt`
- author post-fix regression — semantic_contract; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI; passed=58, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/6f41024540-semantic-review-fix-swift.txt`
- independent original-source pre-fix reproduction — state_receipts; Production TypeScript reduce receive/adoption boundary, direct pure function tests. Not browser/native transport E2E or Swift coverage.; passed=2, failed=5. `docs/releases/evidence/2026-09-06-convergence/final/review-closeout/n1-original-verify/baseline-ts.txt`
- independent post-fix verification — state_receipts; Production TypeScript reduce receive/adoption boundary, direct pure function tests. Not browser/native transport E2E or Swift coverage.; passed=7, failed=0. `docs/releases/evidence/2026-09-06-convergence/final/review-closeout/n1-original-verify/current-ts.txt`
- Both trees are exact git archive extracts. Production source is byte-identical to the named commit, not a mutation or reverse patch. Root Cargo.toml workspace member list is narrowed to the three pure crates as harness setup only; tests/config are independently added outside shared checkout.
- Destructive patch deletes required members rather than copying author original mood-deletion test. Inputs and control cases authored independently.
- Fix landed in 30ff852; independent final reproduction tested exact archived 3b375bd bytes. These identities are intentionally distinct.

### N1-RESTORE-02 — Authoritative snapshot restore silently discarded explicit null before validation

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: independently verified original defect; fixed behavior confirmed.

- author pre-fix regression — semantic_contract; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI; passed=0, failed=1. `docs/releases/evidence/2026-09-06-convergence/review/d2cb866c5e-semantic-host-null-red.txt`
- author post-fix regression — semantic_contract; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI; passed=5, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/32dfb8961a-semantic-review-fix-rust.txt`
- independent original-source pre-fix reproduction — state_receipts; Production CharacterSession::restore wrapper and restore_report, actual serde/canonical implementation from three pure crates. No filesystem storage adapter/daemon/native UI exercised.; passed=2, failed=1. `docs/releases/evidence/2026-09-06-convergence/final/review-closeout/n1-original-verify/baseline-rust.txt`
- independent post-fix verification — state_receipts; Production CharacterSession::restore wrapper and restore_report, actual serde/canonical implementation from three pure crates. No filesystem storage adapter/daemon/native UI exercised.; passed=3, failed=0. `docs/releases/evidence/2026-09-06-convergence/final/review-closeout/n1-original-verify/current-rust-clean-target.txt`
- Both trees are exact git archive extracts. Production source is byte-identical to the named commit, not a mutation or reverse patch. Root Cargo.toml workspace member list is narrowed to the three pure crates as harness setup only; tests/config are independently added outside shared checkout.
- Restore controls demonstrate hash/deny-unknown checks did not protect against raw null before serde.
- Fix landed in 30ff852; independent final reproduction tested exact archived 3b375bd bytes. These identities are intentionally distinct.
- Excluded current-rust.log reused the baseline build artifact due to historical mtimes; only current-rust-clean-target.log from a fresh target counts as fixed-source verification. This is a harness provenance issue, not an unfixed product result.

### N1-REVIEW-03 — Swift flattened Attention DTO rejected a valid unknown-variant field

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- independent pre-fix reproduction — root; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI. `docs/releases/evidence/2026-09-06-convergence/review/4d23949821-n1-ts-verify.txt`
- independent pre-fix reproduction — root; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI. `docs/releases/evidence/2026-09-06-convergence/review/d40ac2a2a1-n1-swift-verify.txt`
- independent post-fix verification — root; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI. `docs/releases/evidence/2026-09-06-convergence/review/f5029de6b3-n1-rust-verify.txt`
- independent post-fix verification — root; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI. `docs/releases/evidence/2026-09-06-convergence/review/5da40d8951-n1-ts-verify-fixed.txt`
- independent post-fix verification — root; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI. `docs/releases/evidence/2026-09-06-convergence/review/104fbf0142-n1-swift-verify-fixed.txt`
- Report names root as formal finder/reproducer of five probes. Earlier conversation candidate came from restart_lifecycle; root expanded date probes to leap-second case. Semantic author confirmed report is not exclusive proof of earliest candidate attribution.

### N1-REVIEW-04 — Consumer date validation normalized invalid dates and rejected valid leap seconds

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- independent pre-fix reproduction — root; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI. `docs/releases/evidence/2026-09-06-convergence/review/f5029de6b3-n1-rust-verify.txt`
- independent pre-fix reproduction — root; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI. `docs/releases/evidence/2026-09-06-convergence/review/4d23949821-n1-ts-verify.txt`
- independent pre-fix reproduction — root; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI. `docs/releases/evidence/2026-09-06-convergence/review/d40ac2a2a1-n1-swift-verify.txt`
- independent post-fix verification — root; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI. `docs/releases/evidence/2026-09-06-convergence/review/5da40d8951-n1-ts-verify-fixed.txt`
- independent post-fix verification — root; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI. `docs/releases/evidence/2026-09-06-convergence/review/104fbf0142-n1-swift-verify-fixed.txt`
- independent post-fix verification — root; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI; passed=59, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/1c84d147ba-semantic-review-fix-ts.txt`
- independent post-fix verification — root; Rust pure authoritative model / TS consumer model / native macOS Swift model; no device or UI; passed=58, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/6f41024540-semantic-review-fix-swift.txt`
- Report names root as formal finder/reproducer of five probes. Earlier conversation candidate came from restart_lifecycle; root expanded date probes to leap-second case. Semantic author confirmed report is not exclusive proof of earliest candidate attribution.

### N4-REVIEW-01 — Import reported settings unchanged after prefs had already committed

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- finder pre-fix reproduction — semantic_contract; actual React import boundary and/or pure validation, API promises controlled; passed=0, failed=1, skipped=24. `docs/releases/evidence/2026-09-06-convergence/review/f51066142f-n4-import-review-red.txt`
- independent pre-fix reproduction — root; actual React import boundary and/or pure validation, API promises controlled; failed=1. `docs/releases/evidence/2026-09-06-convergence/review/1b0fe1b190-n4-import-independent-red.txt`
- independent post-fix verification — semantic_contract; actual React import boundary and/or pure validation, API promises controlled; passed=41, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/0ac2308ac0-n4-review-final-independent-green.txt`

### N4-REVIEW-02 — Atomic preference replacement widened an existing private file mode

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- finder pre-fix reproduction — semantic_contract; real production Rust preference writer, isolated filesystem fixture; passed=0, failed=2. `docs/releases/evidence/2026-09-06-convergence/review/a07b720669-n4-prefs-writer-review-red.txt`
- independent pre-fix reproduction — root; real production Rust preference writer, isolated filesystem fixture; failed=2. `docs/releases/evidence/2026-09-06-convergence/review/8500c4cbdf-n4-writer-independent-red.txt`
- independent post-fix verification — semantic_contract; real production Rust preference writer, isolated filesystem fixture; passed=2, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/81e059580a-n4-writer-review-final-independent-green.txt`
- independent post-fix verification — semantic_contract; real production Rust preference writer, isolated filesystem fixture; passed=2, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/f9197a54fa-n4-writer-green.txt`

### N4-REVIEW-03 — Failed temporary create removed a pre-existing file the writer did not own

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- finder pre-fix reproduction — semantic_contract; real production Rust preference writer, isolated filesystem fixture; passed=0, failed=2. `docs/releases/evidence/2026-09-06-convergence/review/a07b720669-n4-prefs-writer-review-red.txt`
- finder pre-fix reproduction — semantic_contract; real production Rust preference writer, isolated filesystem fixture; passed=0, failed=1. `docs/releases/evidence/2026-09-06-convergence/review/5179e2889a-n4-prefs-collision-review-red.txt`
- independent pre-fix reproduction — root; real production Rust preference writer, isolated filesystem fixture; failed=2. `docs/releases/evidence/2026-09-06-convergence/review/8500c4cbdf-n4-writer-independent-red.txt`
- independent post-fix verification — semantic_contract; real production Rust preference writer, isolated filesystem fixture; passed=2, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/81e059580a-n4-writer-review-final-independent-green.txt`
- Root raised the temp collision suspicion; semantic_contract supplied the concrete reproduction. Keep initial harness issue separate from dedicated valid collision log.

### N4-REVIEW-04 — Nested malformed backup fields were coerced into persisted strings

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- finder pre-fix reproduction — semantic_contract; actual React import boundary and/or pure validation, API promises controlled; passed=0, failed=4. `docs/releases/evidence/2026-09-06-convergence/review/f7e38d96d2-n4-nested-backup-review-red.txt`
- independent pre-fix reproduction — root; actual React import boundary and/or pure validation, API promises controlled; failed=4. `docs/releases/evidence/2026-09-06-convergence/review/f219b0a57c-n4-nested-independent-red.txt`
- independent post-fix verification — semantic_contract; actual React import boundary and/or pure validation, API promises controlled; passed=41, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/0ac2308ac0-n4-review-final-independent-green.txt`

### N6-REVIEW-01 — Initial Shift+Tab escaped a newly opened modal and lost its Escape handler

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- independent pre-fix reproduction — semantic_contract; real Chromium UI plus production daemon fixture; supplemental jsdom focus test; passed=0, failed=1. `docs/releases/evidence/2026-09-06-convergence/review/565f9030a8-n6-dialog-browser-independent-red.txt`
- independent pre-fix reproduction — semantic_contract; real Chromium UI plus production daemon fixture; supplemental jsdom focus test; passed=6, failed=1. `docs/releases/evidence/2026-09-06-convergence/review/5a0cfc34cf-n6-dialog-independent-unit-red.txt`
- author post-fix regression — semantic_contract; real Chromium UI plus production daemon fixture; supplemental jsdom focus test; passed=1, failed=0, seconds=5.9. `docs/releases/evidence/2026-09-06-convergence/review/389b29cd60-n6-dialog-browser-green.txt`
- author post-fix regression — semantic_contract; real Chromium UI plus production daemon fixture; supplemental jsdom focus test; passed=71, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/6eac3bdcab-n6-dialog-shared-unit-green.txt`
- independent post-fix verification — root; full Chromium browser suite with real daemon and fixtures; passed=92, failed=0. `docs/releases/evidence/2026-09-06-convergence/final/final-playwright.txt`
- Independent final Browser covers populated emergency dialog only. Empty loading-overlay trap remains covered by author unit test; do not claim root Browser exercised empty dialog.

### N4-UI-01 — A pre-operation poll overwrote a successfully completed preset

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- independent pre-fix reproduction — /root/state_receipts; actual React component with controlled desktop-port promises; passed=0, failed=2, skipped=13, durationSeconds=2.78. `docs/releases/evidence/2026-09-06-convergence/review/9067a949c2-n4-independent-verify-red.txt`
- independent post-fix verification — /root/state_receipts; actual React component with controlled desktop-port promises; passed=16, failed=0, skipped=0, durationSeconds=2.71. `docs/releases/evidence/2026-09-06-convergence/review/18debc34a5-n4-independent-final-green.txt`
- runHostPreset now invalidates earlier preferences reads when the operation starts.
- React component plus controlled desktop promises/events. Native host crash/reconnect walkthrough evidence belongs to parent.

### N4-UI-02 — Reconnect recovery left preset/proactive readback stale

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- independent pre-fix reproduction — /root/state_receipts; actual React component with controlled desktop-port promises; passed=0, failed=2, skipped=13, durationSeconds=2.78. `docs/releases/evidence/2026-09-06-convergence/review/9067a949c2-n4-independent-verify-red.txt`
- independent post-fix verification — /root/state_receipts; actual React component with controlled desktop-port promises; passed=16, failed=0, skipped=0, durationSeconds=2.71. `docs/releases/evidence/2026-09-06-convergence/review/18debc34a5-n4-independent-final-green.txt`
- Host result events and connection/refresh invalidations now reload preferences and proactive state; the actual component reconciles an unverified operation after reconnect.
- React component plus controlled desktop promises/events. Native host crash/reconnect walkthrough evidence belongs to parent.

### N4-UI-03 — A poll issued during a host operation overwrote its later completion

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- finder pre-fix reproduction; parent independent reasoning reported — /root/state_receipts; actual React component with controlled desktop-port promises; passed=0, failed=1, skipped=15, durationSeconds=1.69. `docs/releases/evidence/2026-09-06-convergence/review/edaac44980-n4-overlapping-read-verify.txt`
- independent post-fix verification — /root/state_receipts; actual React component with controlled desktop-port promises; passed=16, failed=0, skipped=0, durationSeconds=2.71. `docs/releases/evidence/2026-09-06-convergence/review/18debc34a5-n4-independent-final-green.txt`
- Start-only generation invalidation did not cover a prefsGet issued after start. Completion now advances the generation before exposing the final preset response; a late read cannot replace it.
- Independent added delayed-promise regression; parent implements fix and broader suite. Not an OS-scheduler race stress test.

### N3-SCOPE-01 — Changing a family capture list overwrote previous unresolved sensor evidence

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- finder pre-fix reproduction — /root/semantic_contract; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/a130abed81-semantic-n3-review-red.txt`
- independent pre-fix reproduction — /root/state_receipts; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/05108a8a15-n3-independent-verify-red.txt`
- independent post-fix verification — state_receipts; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/2297cda2e5-n3-independent-final-green.txt`
- author post-fix full scoped regression — /root/restart_lifecycle; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/28641b49ac-n3-mobile-observed-final.txt`
- independent latest post-fix verification — state_receipts; independent production SensorSource port fixture; passed=5, failed=0, seconds=0.19. `docs/releases/evidence/2026-09-06-convergence/review/6aeaca3b23-n3-journal-latest-independent-green.txt`
- Union old and new (sensor, scope) evidence for the same source generation; preserve earliest since and old evidence first under the bounded cap. A changed live list is never negative stop evidence.

### N3-SCOPE-02 — A current family member's stopped report cleared an absent member of the same sensor kind

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- finder pre-fix reproduction — /root/semantic_contract; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/a130abed81-semantic-n3-review-red.txt`
- independent pre-fix reproduction — /root/state_receipts; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/05108a8a15-n3-independent-verify-red.txt`
- independent post-fix verification — state_receipts; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/2297cda2e5-n3-independent-final-green.txt`
- author post-fix full scoped regression — /root/restart_lifecycle; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/28641b49ac-n3-mobile-observed-final.txt`
- independent latest post-fix verification — state_receipts; independent production SensorSource port fixture; passed=5, failed=0, seconds=0.19. `docs/releases/evidence/2026-09-06-convergence/review/6aeaca3b23-n3-journal-latest-independent-green.txt`
- Resolve only same process/source generation plus exact adapter capture scope and sensor. Mobile supplies the connection ID captured with the capture/stop target. Default family scopes cannot be vouched for by another member report. New connection with the same device ID cannot clear old scope.

### N3-SCOPE-03 — A registered family hid historical unknowns after its live capture disappeared

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- author pre-fix regression — /root/restart_lifecycle; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/f28138bb79-n3-live-family-red.txt`
- independent pre-fix reproduction — /root/state_receipts; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/05108a8a15-n3-independent-verify-red.txt`
- independent post-fix verification — state_receipts; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/2297cda2e5-n3-independent-final-green.txt`
- author post-fix full scoped regression — /root/restart_lifecycle; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/28641b49ac-n3-mobile-observed-final.txt`
- independent latest post-fix verification — state_receipts; independent production SensorSource port fixture; passed=5, failed=0, seconds=0.19. `docs/releases/evidence/2026-09-06-convergence/review/6aeaca3b23-n3-journal-latest-independent-green.txt`
- Hide only exact currently represented (sensor, scope) evidence, or the existing 60-second orphan projection window. Constant family registration alone never suppresses historical unknowns. Human dismissal removes only visible historical evidence and retains live siblings.
- Concrete live-only projection finding was first raised by the implementation author. /root/semantic_contract confirmed this attribution from the conversation; only scope01/02 were independently found by that agent. /root/state_receipts independently refuted/verified all three against the actual production path and reran 0/3 red. Do not relabel the scope03 author extension as an independent Find.

### N3-MOBILE-EDGE-01 — Per-device stop and direct mobile removal bypassed durable capture evidence

Classification: product defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: recorded.

- finder pre-fix reproduction — /root/state_receipts; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/8b44aa47fd-n3-mobile-direct-stop-red.txt`
- independent pre-fix reproduction — /root; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/6fd632aad7-n3-mobile-independent-red.txt`
- author post-fix full scoped regression — /root/restart_lifecycle; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/28641b49ac-n3-mobile-observed-final.txt`
- independent post-fix verification — state_receipts; real Runtime ports with controlled family/TLS peer and persistent store. `docs/releases/evidence/2026-09-06-convergence/review/ccf7d4f897-n3-mobile-direct-stop-independent-green.txt`
- Capture raw false-to-true through the canonical journal port before publishing mic_since. Each connection carries opaque SensorCaptureOwner with registered source generation and family label. Same effective connection's false report can settle only its scope while the source generation is still registered. Teardown never clears it; direct per-device stop now uses the canonical source request when registered. Safety-decreasing raw wire stop remains available if source registration has already been removed.

### N3-RESTART — requested restart

Classification: user-requested contract delta, not counted as independently found defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: author delta explicitly distinguished from independent finding.

- author pre-fix delta regression — restart_lifecycle; real Runtime and persistent store fixture. `docs/releases/evidence/2026-09-06-convergence/review/af40dbe191-n3-restart-red.txt`
- author post-fix scoped regression — restart_lifecycle; real Runtime and persistent store fixture. `docs/releases/evidence/2026-09-06-convergence/review/28641b49ac-n3-mobile-observed-final.txt`
- Original red was 1 pass / 2 fail: historical reminder retention failed both after and before orphan TTL. This is not claimed as an independent Find/Verify run.
- Parent explicitly accepts cross-scope independent review; do not invent an independent original red for this requested delta.

### N3-GENERATION — requested generation

Classification: user-requested contract delta, not counted as independently found defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: author delta explicitly distinguished from independent finding.

- author pre-fix delta regression — restart_lifecycle; real Runtime and persistent store fixture. `docs/releases/evidence/2026-09-06-convergence/review/200f14f45d-n3-generation-red.txt`
- author post-fix scoped regression — restart_lifecycle; real Runtime and persistent store fixture. `docs/releases/evidence/2026-09-06-convergence/review/28641b49ac-n3-mobile-observed-final.txt`
- v0.7.0 explicitly allowed a new same-ID source to clear an old generation. The new red establishes the requested stricter rule; it is not evidence that v0.7.0 already promised this same rule. Compatibility/deprecation/privacy docs record the behavior change.
- Parent explicitly accepts cross-scope independent review; do not invent an independent original red for this requested delta.

### N3-SHUTDOWN — requested shutdown

Classification: user-requested contract delta, not counted as independently found defect. Commit: `30ff852f5d893215c019d982774c8b59ab010659`. Verification: author delta explicitly distinguished from independent finding.

- author pre-fix delta regression — restart_lifecycle; real Runtime and persistent store fixture. `docs/releases/evidence/2026-09-06-convergence/review/e3c9732b98-n3-shutdown-red.txt`
- author post-fix scoped regression — restart_lifecycle; real Runtime and persistent store fixture. `docs/releases/evidence/2026-09-06-convergence/review/28641b49ac-n3-mobile-observed-final.txt`
- Shutdown now stops sensors through the existing bounded coordinator before transport cancellation and journal flush/clean marker. Synchronous local microphone stop remains first.
- Parent explicitly accepts cross-scope independent review; do not invent an independent original red for this requested delta.

### N3-E2E-SENSOR-TEARDOWN — checkpoint Playwright sensors.spec.ts:112 expected .sensor-banner count 0, actual 1; success message assertion at 111 already passed

Classification: fixture isolation defect. Commit: `87b0d03173cdefce68bad2403b7f76be59b738e1`. Verification: recorded.

- finder differential control — restart_lifecycle; real Chromium UI + daemon + external simulated phone; passed=1, failed=0, durationSeconds=7.5. `docs/releases/evidence/2026-09-06-convergence/review/859ca850b0-n3-sensors-isolated-18890.txt`
- finder pre-fix reproduction — restart_lifecycle; real Chromium UI + daemon + external simulated phone; passed=1, failed=1, notRun=1, durationSeconds=27.1. `docs/releases/evidence/2026-09-06-convergence/review/93dd1dbde9-n3-sensors-sequence-red.txt`
- author fixture post-fix regression — restart_lifecycle; real Chromium UI + daemon + external simulated phone; passed=3, failed=0, durationSeconds=11.8. `docs/releases/evidence/2026-09-06-convergence/review/2fa1a0647c-n3-sensors-sequence-green.txt`
- independent full-suite post-fix verification — root; real Chromium UI + daemon + external simulated phone; passed=92, failed=0. `docs/releases/evidence/2026-09-06-convergence/final/final-playwright.txt`
- Before ending each still-live fixture, send its genuine same-connection status micLevel=false and poll activeSensors and unresolvedStops empty. Always kill/revoke in finally. Do not dismiss, delete or suppress unknowns. If cleanup cannot confirm, teardown fails.
- process-outside simulated iPhone fixture against real daemon + real browser UI; not physical iPhone acceptance

### N3-UI-STOP-HISTORY — Old mobile capture disconnects without confirmation; a new current capture stops successfully. Current reports are all confirmed and activeSensors empty, but historical unresolvedStops or journal recovery health remains. UI previously emitted the general successful 已停止感測。 notice alongside the unresolved reminder.

Classification: product defect. Commit: `87b0d03173cdefce68bad2403b7f76be59b738e1`. Verification: recorded.

- author pre-fix regression — restart_lifecycle; pure shared projection and actual App/Home/Search component with mocked transport; passed=0, failed=9, skipped=81, durationSeconds=3.77. `docs/releases/evidence/2026-09-06-convergence/review/af92dc3736-n3-ui-stop-history-final-red.txt`
- author post-fix regression — restart_lifecycle; pure shared projection and actual App/Home/Search component with mocked transport; passed=111, failed=0, durationSeconds=2.9. `docs/releases/evidence/2026-09-06-convergence/review/abc5d3b154-n3-ui-stop-history-green.txt`
- independent pre-fix isolated reversal — root; isolated/current jsdom application regression; passed=0, failed=9, skipped=81, durationSeconds=4.817. `docs/releases/evidence/2026-09-06-convergence/review/7702cece80-n3-ui-stop-history-independent-red.txt`
- independent post-fix verification — root; isolated/current jsdom application regression; passed=9, failed=0, skipped=81, durationSeconds=2.49. `docs/releases/evidence/2026-09-06-convergence/review/5c85eb515e-n3-ui-stop-history-independent.txt`
- All three production callers pass projectUnresolvedStops(freshStatus).summary to the shared projectSensorStop. A non-null reminder, including overflow/recovery/parked/write-failure health, returns ok=false and a plain warning. Existing live-capture, uncertain-report and status-fetch-failure branches retain their earlier more specific warnings. Journal data remains unchanged.
- Vitest tests mock API/desktop transport but execute actual shared projection and UI handlers. Full initial Browser/network trace independently establishes reachability. Does not claim physical/native acceptance of the updated notice.

### SCRIPT-01 — XCTest accepted fewer tests and reused an earlier pass after incomplete output

Classification: validation harness defect. Commit: `87b0d03173cdefce68bad2403b7f76be59b738e1`. Verification: recorded.

- finder/author pre-fix parser regression — semantic_contract; pure Python production parser fixture; passed=10, failed=4. `docs/releases/evidence/2026-09-06-convergence/review/826cc1dad0-xctest-result-gate-red.txt`
- independent pre-fix parser reproduction — root; pure Python production parser probe. `docs/releases/evidence/2026-09-06-convergence/review/a1cecd8f12-xctest-independent-red.txt`
- author post-fix regression — semantic_contract; pure Python production parser fixture; passed=14, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/9ca6a0d581-xctest-result-gate-green.txt`
- independent post-fix regression — root; pure Python production parser fixture; passed=14, failed=0. `docs/releases/evidence/2026-09-06-convergence/final/xctest-gate-independent.txt`
- All tests passed / Executed1 test returned exit0 despite this round expecting163; a subsequent Alltests started without completion also reused old success.
- Explicit --expected-count default163 in parser/runner; exact cohort, latest complete Alltests suite, no failures/skips or trailing incomplete output. Failure assertions and failed method counts are distinguished.
- Parent supplied existing independent green missing from the handoff wording. No new XCTest/native run was performed by this auditor.

### SCRIPT-02 — Native binary identity was recorded at the end rather than pinned before execution

Classification: validation harness defect. Commit: `87b0d03173cdefce68bad2403b7f76be59b738e1`. Verification: recorded.

- independent post-fix extracted production guard probes — semantic_contract; Pure in-memory execution of each unchanged production provenance guard; no native app execution; passed=4, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/6ea043959d-validation-script-review-probes.json`
- Run executableA, rebuild/replace path withB during long walkthrough; old final-only hashing attributed the earlyA steps toB. This was a source-proven conditional risk, not an observed change during the frozen integration run.
- Capture source/dirty/binary/CLI/driver identity before launch and reject changed binary/CLI hashes on completion.
- Pre-fix defect has reported source/conditional probe evidence, not a separately archived independent red-test run. Post-fix pure guard execution is independent of root author and does not execute native/perf.

### SCRIPT-03 — Download selection could claim and delete another concurrent matching backup

Classification: validation harness defect. Commit: `87b0d03173cdefce68bad2403b7f76be59b738e1`. Verification: recorded.

- finder conditional reproduction and independent post-fix selector probes — semantic_contract; Pure in-memory invocation of the production exported() function and cleanup loop; actual Downloads untouched; passed=3, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/validation-scripts-review.json`
- After prior snapshot, another process creates companion-settings*.json with fixed companionName=備份走查 while this app creates no download. Extracted production selector chose that file and cleanup unlinked it in a pure in-memory fake-path reproduction.
- Random per-run companion name and record device/inode/content digest; delete only the unchanged selected file.
- Three selector observations are preserved in the archived review JSON, without a separate raw CLI transcript. No real Downloads deletion was used for this proof.

### SCRIPT-04 — Performance wrapper ignored explicit evidence-quality failure flags

Classification: validation harness defect. Commit: `87b0d03173cdefce68bad2403b7f76be59b738e1`. Verification: recorded.

- independent post-fix extracted production guard probes — semantic_contract; Production artifact-guard loop extracted unchanged and executed over pure in-memory JSON; three bad cases plus good control, no perf run; passed=4, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/6ea043959d-validation-script-review-probes.json`
- memorySoak.evidenceGrade=false, gcAvailable=false or looksQuantized=true with otherwise good metrics/delta0 left investigate empty. Underlying soak implementation explicitly says quantized values cannot count as memory evidence.
- Non-evidence-grade, unavailableGC or quantized readings enter investigate and prevent a success exit.
- Pre-fix defect has reported source/conditional probe evidence, not a separately archived independent red-test run. Post-fix pure guard execution is independent of root author and does not execute native/perf.

### DOC-01 — Architecture diagram still placed preset recovery ownership in presentation

Classification: documentation defect. Commit: `87b0d03173cdefce68bad2403b7f76be59b738e1`. Verification: recorded.

- independent post-fix static/source verification — semantic_contract; document/source inspection only; Independent current-file read confirms docs/ARCHITECTURE.md:202 and208; MAINTAINERS-MAP.md §8 was already correct.. `docs/releases/evidence/2026-09-06-convergence/review/validation-scripts-review.json`
- docs/ARCHITECTURE.md called applyPresetPlan a recoverable two-stage transaction in presentation and omitted preset_service from application usecases.
- Presentation keeps pure planning/result projection; host preset_service owns conditional writes/readback/restart recovery.

### DRIVER-01 — New random backup marker exceeded the existing24-character name limit

Classification: validation harness defect. Commit: `87b0d03173cdefce68bad2403b7f76be59b738e1`. Verification: recorded.

- independent post-fix static/source verification — semantic_contract; document/source inspection only; Current source has uuid.uuid4().hex[:16]; within existing import limit. No native UI run claimed.. `docs/releases/evidence/2026-09-06-convergence/review/validation-scripts-review.json`
- Initial fix used 備份走查- plus32hex=37characters; valid restore overrides name, but missing-character case keeps overlong backup name and would fail at name validation before the intended package-missing error.
- UUID suffix shortened to16hex, total21characters.

### native-result-gate-emergency-status — Native result gate exempted failed/not-run emergency unlock records

Classification: validation harness defect. Commit: `3b375bde4188e4a039852881d48d82ea9b619ce1`. Verification: recorded.

- independent finder pre-fix exact guard probes — semantic_contract; production Python exit predicate extracted from native shell; no native UI; passed=5, failed=2. `docs/releases/evidence/2026-09-06-convergence/review/e834b38af6-final-native-result-gate-probes.json`
- independent post-fix exact guard probes — semantic_contract; production Python exit predicate extracted from native shell; no native UI; passed=7, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/370973d17c-final-native-result-gate-green.json`
- Normal driver records needs-environment explicitly; this finding does not invalidate the existing completed native run. The gate would miss an absent/failed emergency record.
- Source report verificationCheckpoint 87b0d03 is base plus explicit patch, not fix commit. git diff proves actual fix landed in 3b375bd.

### ARCH-SWIFT-BOUNDARY — Unbraced shell variable absorbed adjacent full-width punctuation

Classification: validation harness defect. Commit: `87b0d03173cdefce68bad2403b7f76be59b738e1`. Verification: recorded.

- independent finder original runner failure — root; native macOS Swift models pass but shell aggregate fails. `docs/releases/evidence/2026-09-06-convergence/review/d9c59e1a10-checkpoint-architecture.txt`
- fix-author fixed runner regression — state_receipts; native macOS Swift model + aggregate shell runner; passed=58, failed=0. `docs/releases/evidence/2026-09-06-convergence/review/baa07b6230-architecture-swift-boundary-independent.txt`
- independent clean-checkpoint post-fix verification — root; native macOS Swift pure model; exact3b375bd clean checkpoint; passed=58, failed=0. `docs/releases/evidence/2026-09-06-convergence/final/architecture-clean-checkpoint/swift-native.txt`
- independent aggregate runner post-fix verification — root; architecture --docs --ts --rust --swift --drill-lint --drills; parent executed, auditor only read; seconds=190.36. `docs/releases/evidence/2026-09-06-convergence/final/clean-checkpoint-architecture.txt`
- At architecture-checks.sh:306, unbraced $SWIFT_NOTE adjacent to full-width punctuation was parsed as a longer variable under nounset. Native Swift body had passed 58/0 but aggregate runner failed.
- Only ${SWIFT_NOTE} explicit variable boundary.
- The earlier handoff labels its green independent while the executor is the fixer state_receipts. It is conservatively labeled author green here. Root exact3b clean architecture run independently proves Swift58/0 and aggregate exit0.

### N2-E2E-LEGACY-01 — Legacy phone fixture expected a stronger applied status than it negotiated

Classification: outdated test expectation; not product regression. Commit: `87b0d03173cdefce68bad2403b7f76be59b738e1`. Verification: recorded.

- independent integration discovery — root; Chromium browser UI + production daemon + external fake_iphone simulator, not native Tauri or real phone; passed=80, notRun=7. `docs/releases/evidence/2026-09-06-convergence/review/908abfc8f7-checkpoint-playwright.txt`
- fix author scoped Browser regression — state_receipts; Chromium browser UI + production daemon + external fake_iphone simulator, not native Tauri or real phone; passed=8, failed=0, seconds=41.5. `docs/releases/evidence/2026-09-06-convergence/review/8085bf57b3-n2-e2e-legacy-green.txt`
- independent complete Browser verification — root; Chromium browser UI + production daemon + external fake_iphone simulator, not native Tauri or real phone; passed=92, failed=0, seconds=222.34. `docs/releases/evidence/2026-09-06-convergence/final/final-playwright.txt`
- Committed fake_iphone capability declares haptic/reducedMotion but not stateApplied, and implements no validated application receipt. Old headline allowlists excluded syncing; general-mode even allowed synced. Exact syncing is the correct negotiated-contract result.
- General task6 removal included as actual predecessor for task7; all four character cases included to run the three earlier serial skips. No native UI operated. Old four failures were observed in parent full-suite log; no new production defect inferred.
- Four old failures are one stale contract issue; stronger diagnostics assertions added, no fake applied receipt or weakened production gate.

## Integrity and limits

- Raw log manifest: 107 distinct archived files, all recorded archived SHA-256 hashes match; no missing exact main-report review-file reference.
- Original SHA and archived SHA are separate because archival normalization/redaction may change bytes. The JSON records current artifact hashes.
- Repeated suites and findings sharing the same logs are not additive unique test counts.
- Independent pre-fix source reasoning is not silently upgraded to an independently executed regression; author red is labeled explicitly.
- Requested N3 restart/generation/shutdown contract deltas are listed separately and are not invented independent findings.
- Root final Browser: 92/0, 222.34s; relevant Dialog case at log line150 passed in1.0s. This covers initial Shift+Tab/Escape/focus return, not empty-overlay Browser behavior.
- SCRIPT-01 independent postfix14/0 log supplied by parent resolves the handoff wording gap.
- N1 independent original archive verification: TS2 controls/5 failures ->7/0; Rust2 controls/1 null failure ->3/0. All15 source/probe/log artifacts match their SHA256; stale reused-target Rust attempt is explicitly excluded.
- ARCH-SWIFT-BOUNDARY earlier fixer-run green is author evidence; root clean3b architecture Swift58/0 plus aggregate exit0 provides independent postfix.
- Simulator/pty/TLS and jsdom proof do not establish physical iPhone/ESP32 acceptance. Native/perf final gates and release remain parent-owned.
- Latest externally supplied logs may still be under `/tmp`; parent should archive them and rewrite references when archiving this map.
