# release-prepare independent review

**Passed; 0 findings.** Reviewer: semantic_contract. Read-only committed-tree comparison `3616c241fee8bb9b48717b21a31a1ad3e84fd014` → **`c15873acbaaed870d2c90fa95c12255f532ce782`**. No repo edits, builds or heavy tests.

| Check | Result |
|---|---|
| Four manifests | Only product version 0.7.0 → 0.8.0; all agree |
| Root Cargo.lock | 20 internal package versions only; external dependencies/checksums/edges unchanged |
| Tauri Cargo.lock | 19 internal package versions only; external dependencies/checksums/edges unchanged |
| OpenAPI | Only `/info/version` → 0.8.0; OpenAPI3.1.0 and every operation/schema unchanged |
| CHANGELOG | Added dated 0.8.0 heading beneath Unreleased; existing release body and limitations byte unchanged |
| Frozen v0.7.0 | Manifest plus five files unchanged; all five equal their declared SHA256 and original published-source blobs |
| Production code/tests/generated consumers | No changes in this prepare range |
| Remaining limits | No downgrade or new acceptance claim |

The delta has **10 files**, not only the eight release outputs: `docs/releases/evidence-index.json` and `docs/releases/next-convergence-progress.md` also record preparation status. Their change still says final verification/tag are pending. The recorded40.51s preparation duration comes from root's execution record; this review did not retime it.

Product0.8.0 and Rust crate package versions are separate from AIP `aip/1.0`, `semantic-state/1.0`, CPP1.0 and Session snapshot format1, all unchanged. Device keeps `proto=1`; device wirev1.3/mobile wirev1.1 add only the negotiated `aip.applied/1` profile. Existing fragmentationv1.2 constraints and legacy unconfirmed behavior remain. Frozen source is `630b4291f6a59444cfb1d8185f757f9dcda9ecc4`; it was not regenerated into product0.8.0 samples.

Limitations retain real iPhone/ESP32/human usability gaps; unverified MQTT/BLE-specific applied loops; reference firmware without fragmentation/applied; receipts as peer self-report; non-reconstructable historical unknown stops; conservative overflow/future-state treatment; human emergency unlock. Full candidate matrix, merge, main CI, tag and release are separate pending gates and are not certified by this diff review.

[Machine-readable checks and per-file hashes](release-prepare-independent-review.json). Reproduce the inspected patch with `git diff 3616c241fee8bb9b48717b21a31a1ad3e84fd014 c15873acbaaed870d2c90fa95c12255f532ce782`. Lightweight checks were parsed JSON/TOML equality, blob SHA256 comparisons and `git diff --check`; tests executed:0.
