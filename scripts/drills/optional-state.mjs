#!/usr/bin/env node
// A disposable extension exercise. Defaults to committed HEAD; --working-tree
// is useful before the integrating commit and is labelled explicitly in evidence.
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync, mkdirSync, cpSync, symlinkSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const temporary = mkdtempSync(join(tmpdir(), "optional-state-drill-"));
const tree = join(temporary, "checkout");
mkdirSync(tree);
const sourceSha = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const source = process.argv.includes("--working-tree") ? "working-tree" : "clean-checkout";
const events = [];
function run(id, command, args, fail = false, env = {}) {
  const start = Date.now();
  const result = spawnSync(command, args, { cwd: tree, env: { ...process.env, CARGO_INCREMENTAL: "0", CARGO_BUILD_JOBS: "4", CARGO_TARGET_DIR: join(temporary, "target"), ...env }, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  const passed = fail ? result.status !== 0 : result.status === 0;
  events.push({ id, command: [command,...args], expected: fail ? "failure" : "success", exitCode: result.status, passed, elapsedMs: Date.now()-start });
  console.log(`${passed ? "PASS" : "FAIL"} ${id} (${Date.now()-start} ms)`);
  if (!passed || result.error) { console.error(result.stderr, result.stdout); throw result.error ?? new Error(id); }
  return `${result.stdout}\n${result.stderr}`;
}
try {
  execFileSync("git",["archive","--output",join(temporary,"source.tar"),"HEAD"],{cwd:root});
  execFileSync("tar", ["-xf", join(temporary,"source.tar"), "-C", tree]);
  rmSync(join(temporary,"source.tar"));
  if (source === "working-tree") {
    const paths = execFileSync("git", ["ls-files", "-co", "--exclude-standard", "-z"], {cwd:root,encoding:"utf8"}).split("\0").filter(Boolean);
    for (const path of paths) if (existsSync(join(root,path))) { mkdirSync(dirname(join(tree,path)), {recursive:true}); cpSync(join(root,path),join(tree,path)); }
  }
  const modules = join(root,"apps/interaction-desktop/node_modules");
  if (!existsSync(modules)) throw new Error("Run pnpm install --frozen-lockfile in apps/interaction-desktop first");
  symlinkSync(modules,join(tree,"apps/interaction-desktop/node_modules"),"dir");
  run("apply-optional-field", "git", ["apply", "scripts/drills/optional-state.patch"]);
  const missingGolden = run("schema-drift-detected", "cargo", ["test","-p","interaction-e2e","--test","golden","golden_semantic_state_schema"], true);
  if (!missingGolden.includes("drifted")) throw new Error("schema test failed for a reason other than drift");
  run("regenerate-schema", "cargo", ["test","-p","interaction-e2e","--test","golden","golden_semantic_state_schema"],false,{GOLDEN_UPDATE:"1"});
  const missingConsumer = run("missing-consumer-detected", "node", ["scripts/aip-codegen.mjs"],true);
  if (!missingConsumer.includes("consumer handling missing")) throw new Error("codegen failed for an unexpected reason");
  const policyPath=join(tree,"schemas/semantic-state-consumers.json");
  const policy=JSON.parse(readFileSync(policyPath,"utf8"));
  policy.fields["/drillOptional"]={typescript:"retained",swift:"retained"};
  writeFileSync(policyPath,JSON.stringify(policy,null,2)+"\n");
  run("propagate-consumer-dtos","node",["scripts/aip-codegen.mjs"]);
  const missingFixture=run("missing-fixture-detected","cargo",["test","-p","interaction-session","--test","state_hash_fixtures","every_semantic_state_field_appears_in_at_least_one_state_hash_fixture"],true);
  if (!missingFixture.includes("drillOptional")) throw new Error("fixture test failed for an unexpected reason");
  const producerPath=join(tree,"crates/interaction-session/tests/state_hash_fixtures.rs");
  const before=readFileSync(producerPath,"utf8");
  const after=before.replace('let fresh = serde_json::to_value(session.state()).expect("state serializes");','let mut fresh = serde_json::to_value(session.state()).expect("state serializes");\n    fresh["drillOptional"] = json!("retained extension");');
  if (after===before) throw new Error("fixture producer anchor changed; update drill consciously");
  writeFileSync(producerPath,after);
  run("regenerate-current-fixtures","cargo",["test","-p","interaction-session","--test","state_hash_fixtures","state_hash_fixtures_are_what_the_host_writes"],false,{AIP_UPDATE_FIXTURES:"1"});
  run("embed-current-fixtures","node",["scripts/aip-codegen.mjs"]);
  run("rust-current-and-published","cargo",["test","-p","interaction-session","--test","semantic_contract","--test","state_hash_fixtures","--test","state_semantics"]);
  run("typescript-consumer","pnpm",["--dir","apps/interaction-desktop","exec","vitest","run","src/test/semantic-state-contract.test.ts","src/test/canonical-hash.test.ts"]);
  run("swift-native-consumer","bash",["scripts/tests/semantic-state-swift.sh"]);
  // Regenerating golden/current fixtures cannot bless a serializer that writes null.
  const domainPath=join(tree,"crates/interaction-session/src/state.rs");
  const domainBefore=readFileSync(domainPath,"utf8");
  const omission='#[serde(default, skip_serializing_if = "Option::is_none")]\n    pub(crate) last_interaction';
  const domainNull=domainBefore.replace(omission,'#[serde(default)]\n    pub(crate) last_interaction');
  if (domainNull===domainBefore) throw new Error("null mutation anchor changed");
  writeFileSync(domainPath,domainNull);
  run("null-mutation-regenerate-schema","cargo",["test","-p","interaction-e2e","--test","golden","golden_semantic_state_schema"],false,{GOLDEN_UPDATE:"1"});
  run("null-mutation-regenerate-fixtures","cargo",["test","-p","interaction-session","--test","state_hash_fixtures","state_hash_fixtures_are_what_the_host_writes"],false,{AIP_UPDATE_FIXTURES:"1"});
  const nullFailure=run("null-mutation-independent-contract","cargo",["test","-p","interaction-session","--test","semantic_contract","schema_never_turns_an_absent_optional_into_a_present_null"],true);
  if (!nullFailure.includes("validate_semantic_state")) throw new Error("null mutation failed for an unexpected reason");
  writeFileSync(domainPath,domainBefore);
  run("null-mutation-repair-fixtures","cargo",["test","-p","interaction-session","--test","state_hash_fixtures","state_hash_fixtures_are_what_the_host_writes"],false,{AIP_UPDATE_FIXTURES:"1"});
  // Published samples are checked by a separate pin, even after current regeneration.
  const frozenPath=join(tree,"crates/interaction-aip/tests/fixtures/releases/v0.7.0/state-hash-fresh.json");
  const frozenBefore=readFileSync(frozenPath,"utf8");
  writeFileSync(frozenPath,frozenBefore+" ");
  const frozenFailure=run("published-corpus-mutation-detected","cargo",["test","-p","interaction-session","--test","semantic_contract","published_v070_corpus_is_immutable_and_still_compatible"],true);
  if (!frozenFailure.includes("f043a8c9")) throw new Error("published corpus mutation failed for an unexpected reason");
  writeFileSync(frozenPath,frozenBefore);
  run("restore-contract-checks","cargo",["test","-p","interaction-session","--test","semantic_contract"]);
  run("embed-repaired-fixtures","node",["scripts/aip-codegen.mjs"]);
  // A DTO accidentally deleting the new field must fail independently of golden regeneration.
  const generated=join(tree,"apps/interaction-desktop/src/aip/semanticStateGenerated.ts");
  writeFileSync(generated,readFileSync(generated,"utf8").replace("  drillOptional?: string;\n",""));
  run("deleted-dto-field-detected","node",["scripts/aip-codegen.mjs","--check"],true);
  run("repair-generated-output","node",["scripts/aip-codegen.mjs"]);
  run("final-codegen-drift","node",["scripts/aip-codegen.mjs","--check"]);
  console.log(JSON.stringify({drill:"optional-state",source,sourceSha,evidenceLevel:"Rust and TS tests plus native Swift pure-model runner; no device evidence",events},null,2));
} finally { rmSync(temporary,{recursive:true,force:true}); }
