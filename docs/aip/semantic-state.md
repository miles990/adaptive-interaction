# Semantic state consumer contract

`interaction-session::SemanticState` owns the domain state. Its derived schema is
`schemas/semantic-state-1.0.schema.json`; `interaction-session::semantic_state_schema()`
exports it without an AIP → Session dependency. `scripts/aip-codegen.mjs` generates
`semanticStateGenerated.ts` and `SemanticStateGenerated.swift` from that schema.
The existing AIP schema and its generated types retain their owners.

## Versions and responsibilities

The product version, AIP `aip/1.0`, device wire profile, `semantic-state/1.0`, and
`SNAPSHOT_FORMAT = 1` describe separate contracts. This local schema formalizes the
existing state fields and omission rule; it does not change the AIP wire version
or snapshot container format. State-applied negotiation is a separate extension
in the transport binding.

| Boundary | Owner | Responsibility |
|---|---|---|
| Authoritative domain state | `interaction-session/src/state.rs` | Only the host produces recognized vocabulary and changes authoritative values |
| Authoritative snapshot restore | `CharacterSession::restore_report` | Validate hash, reject unknown fields/invalid known values and present JSON nulls; preserve existing missing-field migration |
| Consumer wire state | `semantic_contract.rs`, TS `semanticState.ts`, Swift `SemanticStateValidation.swift` | Validate required fields and known field types, retain unknown optional fields and vocabulary without executing unknown behavior |
| Validated state | `ValidatedSemanticState` in all three languages | Bind validation success to the complete original JSON; no coercion or defaulting |
| Renderer projection | TS `projectSemanticState`, Swift `CharacterSemanticState.project` | Read generated DTOs; unknown vocabulary retains its existing unknown presentation; projection never becomes a hash input |

This round also tightens authoritative restore: previously a self-consistent raw
snapshot with `lastInteraction: null` passed serde and the null was silently
dropped. The raw structural rejection now occurs before deserialization, with a
red-to-green regression. This is an implementation correction to the documented
no-null contract, not a claim that the previous host already enforced it.

The two roles are deliberately different: a renderer can retain an unknown field
without acting on it; an authoritative host cannot re-author unrecognized data
from an untrusted saved snapshot. `semantic-state-cases.json` labels
`consumerRole`, `restoreRole`, and each applicable `authoritativeRestore` result.

## Complete states, patches, and numeric representation

A complete state never contains a JSON null. Optional absence is omission;
`"a literal null in a string"` remains a valid string. `null` in an RFC 7396
merge patch deletes the key. The complete merged result is validated before
adoption. Deleting `mood`, for example, rejects the patch without changing the
previous valid state. State bounds come from existing AIP limits: 16 members,
2,000 Unicode scalar values per string/key, 32,768 payload bytes and the payload
nesting bound. The enclosing envelope still has its own independent checks.

Swift keeps `SemanticJSON` number literals. TypeScript keeps ordinary JS values
plus per-container number source metadata in a `WeakMap`. HTTP and SSE parse the
original text; native Tauri uses `character_session_snapshot_raw`,
`character_session_resume_raw`, `events_recent_raw`, and `runtime-event-raw`.
The merge function copies or deletes each numeric source with its value, including
same-valued `1.0 → 1` replacement. Canonical hashing uses the retained source before
known f64 paths. Unknown `9007199254740993` and `1.0` therefore survive hashes and
patches even though a JS numeric projection cannot represent the former exactly.
They must not be interpreted as exact quantities through the projected number.
Host-originated number spellings remain the AIP canonical spellings; the DTO is
never serialized to reconstruct the hash source.

Tagged variant fields are interpreted only within their selected branch. For
example, `attention: {kind: "future-gaze", id: 42}` and
`attention: {kind: "none", id: 42}` retain the extra ID as opaque raw JSON;
the Swift generated reader must not decode that ID using the `member` branch.

Timestamp validation follows the existing Rust chrono RFC3339 reader: Gregorian
month/day and leap-year checks, hours 00–23, minutes 00–59, seconds 00–60, and
offsets up to 23:59. Second 60 is retained as the parser's leap-second form; this
is structural acceptance, not confirmation against a historical leap-second
table. Existing lowercase t/z, space separators and U+2212 negative offset are
preserved for compatibility with that reader. TS/Swift check components directly
so calendar overflow and 24:00 are rejected instead of normalized. These cases
are in the shared corpus, with canonical text and hash pinned independently.

Snapshot, patch, reset, recovery and mixed resume continue through the existing
receive decision table. Connection generation and identity decisions precede
adoption validation. A syntactically invalid candidate that would otherwise be
adopted yields `reject-invalid`; an unrelated/stale connection retains its original
decision. This does not introduce an `allowRegression` option.

## Compatibility evidence and extension exercise

Current fixtures live in `crates/interaction-aip/tests/fixtures`. The shared index
points to `semantic-state-cases.json`; the existing state hash/receive decision
fixtures remain in use. `releases/v0.7.0/` contains five original files copied
byte-for-byte from `630b4291f6a59444cfb1d8185f757f9dcda9ecc4`, with source paths and
checksums. A corpus hash pinned independently in Rust, TypeScript and Swift tests
prevents current fixture/golden regeneration from replacing that release baseline.
The saved snapshot samples include format 0 and the missing `unsupportedIntents`
migration case. They are historical fixture evidence, not new real-device evidence.

Every schema field has an explicit TypeScript/Swift disposition in
`schemas/semantic-state-consumers.json`: `projected` or `retained`. Codegen fails
when a field is added without a consumer decision. Existing independent Rust
field-coverage tests fail when no state hash fixture exercises the new field.
A generated DTO changed by hand fails `aip:check`.

Run the committed extension exercise from a checkout with the usual Rust, Node,
pnpm, installed desktop dependencies and macOS Swift toolchain:

```bash
node scripts/drills/optional-state.mjs
```

It archives committed HEAD into a disposable directory, adds `drillOptional` using
a committed patch, observes schema/consumer/fixture failures, then synchronizes the
contract and fixtures and runs Rust, TypeScript and native Swift behavior checks.
It then changes omission to explicit null and regenerates current artifacts: the
independent contract must still fail. Mutating a published sample must also fail
the separate corpus pin. Both mutations are repaired before the final check.
Only that disposable tree contains the exercise field; it is never added to the
product state. `--working-tree` is a pre-commit development mode and explicitly
labels evidence as such. The script records commands, expected failures, elapsed
milliseconds and source SHA, and removes only its own temporary directory.

Targeted checks:

```bash
cargo test -p interaction-session --test semantic_contract --test state_semantics --test state_hash_fixtures
cargo test -p interaction-e2e --test golden golden_semantic_state_schema
node scripts/aip-codegen.mjs --check
pnpm --dir apps/interaction-desktop exec vitest run src/test/semantic-state-contract.test.ts src/test/semantic-transport.test.ts
bash scripts/tests/semantic-state-swift.sh
```

The native Swift runner compiles actual production model sources and reads the
same files. It proves pure parsing/validation/hash/projection behavior on macOS;
`SemanticStateConformanceTests.swift` additionally exercises the iPhone session
receiver in XCTest. Neither is a substitute for real iPhone UI or hardware evidence.
