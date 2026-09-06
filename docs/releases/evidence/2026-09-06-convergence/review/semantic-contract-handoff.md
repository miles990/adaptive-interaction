# N1 / N5 handoff (2026-09-06)

Implementation complete in shared working tree; no commit/push, no central release/evidence files changed by N1.

N1 domain: new pure interaction-session::semantic_state_schema + borrowed ValidatedSemanticState. Schema generated via e2e golden, DTOs only via aip-codegen, consumer handling ledger must cover every schema path. Required known fields no defaults. Consumer retains bounded unknown optional fields/open vocabulary; authoritative restore remains deny-unknown strict. Raw snapshot null regression reproduced, then fixed before serde can silently omit it. This is a newly enforced implementation correction, not a preexisting guard claim.

Numeric raw path: HTTP/SSE text + Tauri *_raw commands/runtime-event-raw -> TS per-container numeric metadata; merge copies/deletes/replaces metadata, including same-valued1.0->1, unknown large integer and unknown f64 paths. Swift retains SemanticJSON. DTOs/projected fields never become hash inputs. Legacy IPC remains, ledger entry added.

Shared corpus currently44states+4patches. Frozen v0.7.0 corpus5files byte-copied from630b4291f6a59444cfb1d8185f757f9dcda9ecc4; source paths/checksums in manifest; independent corpus pin inRust/TS/Swift never regenerated. Format0/missingunsupportedIntents retained separately from consumer policy.

Independent parent review found and N1 fixed2P2: unknown/none Attention id42 rejected by flattenedSwift DTO; chrono valid leap seconds rejected and invalidcalendar/hour24 normalized by TS/Swift. Codegen now emits selected-variant Swift reader + TS openunion. Explicit date components matchchrono existing structural acceptance (leapsecond60, lowercaset/z, spaceseparator, unicode-minusoffset included); new18sharedcases pin acceptance/rawcanonical/hash.

Evidence (all machine tests/models; no iPhone/ESP32 real hardware):
- Baseline parent Rust1245,Tauri63,TS1816,Swiftsim158 all0failed.
- /tmp/semantic-contract-red.log original5regressions allred before productionfix.
- /tmp/semantic-host-null-red.log actualhost explicitnull restore1red.
- /tmp/semantic-session-all.log fullinteraction-session148passed0failed (before final18fixture additions, sameRust production).
- /tmp/semantic-review-fix-rust.log semantic_contract5passed0failed including44states/4patches/frozen/hostnull,compile2.41s,test0.01s.
- /tmp/semantic-review-fix-ts.log contract54+rawtransport5=59passed0failed,0.879s.
- /tmp/semantic-review-fix-swift.log nativeactualModels58passed0failed (44states+4patches+9existingstatehash+1frozen).
- /tmp/semantic-swift-typecheck-final.log fulliOSApp22files,0errors0warnings; source compiler runner stdout recorded0.
- /tmp/semantic-schema-golden.log generatedschema1passed0failed; /tmp/semantic-codegen-check.log check0.
- /tmp/semantic-optional-drill.log --working-tree mode21stepsallpassed; expectation failures include schema drift, missingconsumer, missingfixture, DTO deletion, omission->null after current regeneration, publishedcorpus mutation. Native40cases during that run (before final18cases). DEFAULT cleanHEAD must be run after integratingcommit; scripts/drills/optional-state.mjs archivesHEAD into disposabletarget, --working-tree clearlylabels precommitmode. N2 will integrate architecture --drills.
- /tmp/semantic-package-native.log productionports1passed0failed,compile25.74s,test0.23s.
- /tmp/semantic-package-drill.log whole node scripts/drills/character-package.mjs passedRust1/0 + TS N5renderer/page2/0,23otherpagecasesfiltered. Records sourceSHA/dirty/commands/time. Fixture examples/characters/drill-text/manifest.json (asset-freeexistingtextadapter). Rustcharacter_store::import/remove + hostpersistprefs + runtimehello + restart. TSnormalfactory/renderersafety, mockedhostpageConfirmButton/remove/fallback/remount. These are separate boundary tests, not a single nativeUIE2E. Root is doingAX native supplementalwalkthrough.

Swift integration: SessionClient owns sendStateApplied default transportmethod and validatedadoptionhook afterstate/resume, tuple/profile/token/generation/hash + exactenvelope messageId/sessionId; N2 owns Protocol/ConnectionManagersender. NewSemanticStateConformanceTests3methods plus SessionClientTests2actualhandleFrame receipt tests need fullSimulatorXCTest. Estimate total163 (baseline158+3+2), actualcount from runner wins.

Docs completed: docs/aip/semantic-state.md, character-session/conformance reference, N1deprecationledgersections; examples/characters/drill-text/README.md explainsnative/modelboundary and no fabricatedremoveaudit. Central MAINTAINERS canonicalmap/evidence/release docs remainrootowner.

Independent review performed:
- N2 excluding own Swift hook: StateAppliedTracker bounded32,monotonic5s,fullrandomtupleandgeneration; protocolpairing/currentrawgeneration, runtimeBound/member, mobilecurrentconn/cancel guards,diagnosticcurrenttuple+tokenredaction. No additional actionablefinding from inspectedpaths. Hardwareevidence remainsnone.
- N3 findings2reproduced independently in crates/interaction-runtime/tests/sensor_journal_review.rs; /tmp/semantic-n3-review-red.log0passed2failed. SameSourceKey newcaptureoverwrite losesA; absentA samekind clearedbyBStopped. N2independentlyVerify0/3inclliveprojectionhidden issue; N3currentlyfixing. Ourreviewtestfile handedtoN3/N2, don't treatitscurrentred asN1regression.

Remaining integration gates: rootindependentN1verifier rerun; SimulatorXCTest; fullchecks; optionaldrilldefaultcleanHEADaftercommit; N3red->greenwithindependentVerify; centraldocs/pr/rebasemerge/releaseverify/tag remotechecks parentowns.
