#!/usr/bin/env node
// Runs from any checkout; all mutable app data belongs to the native test tempdir.
import { execFileSync, spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const sourceSha = execFileSync("git",["rev-parse","HEAD"],{cwd:root,encoding:"utf8"}).trim();
const workingTree = execFileSync("git",["status","--porcelain"],{cwd:root,encoding:"utf8"}).trim().length > 0;
const checks = [
  ["production-host-import-remove-restart", "cargo", ["test","--manifest-path","apps/interaction-desktop/src-tauri/Cargo.toml","--lib","character_package_drill"]],
  ["renderer-and-page-model", "pnpm", ["--dir","apps/interaction-desktop","exec","vitest","run","src/test/character-extension-drill.test.ts","src/test/characterPage.test.tsx","-t","N5"]],
];
const results = [];
for (const [id, command, args] of checks) {
  const start = Date.now();
  const result = spawnSync(command,args,{cwd:root,env:{...process.env,CARGO_INCREMENTAL:"0",CARGO_BUILD_JOBS:"4"},encoding:"utf8",maxBuffer:16*1024*1024});
  results.push({id,command:[command,...args],exitCode:result.status,elapsedMs:Date.now()-start});
  process.stdout.write(result.stdout);
  process.stderr.write(result.stderr);
  if (result.error || result.status !== 0) {
    console.log(JSON.stringify({drill:"character-package",sourceSha,workingTree,results},null,2));
    process.exit(1);
  }
}
console.log(JSON.stringify({drill:"character-package",sourceSha,workingTree,evidenceLevel:"production Rust ports plus TS renderer and mocked-host page; no native UI or device evidence",results},null,2));
