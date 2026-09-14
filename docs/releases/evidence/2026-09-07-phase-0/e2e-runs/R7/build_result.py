import json, pathlib
R = "/private/tmp/claude-501/-Users-user-Workspace-claude-lab-adaptive-interaction/95db6375-b13e-4494-9180-4b15da263657/scratchpad/e2e2/runs/R7"
REPO = "/Users/user/Workspace/claude-lab/adaptive-interaction"
APP = REPO + "/apps/interaction-desktop/src-tauri/target/release/bundle/macos/interaction-control-center.app"

BIN_ID = ("App binary sha256=1103cda7f8491ece2adbaab478056d14a174da47ab8536a2d652c734a0765f19 "
 "(Contents/MacOS/interaction-desktop, built 2026-09-06 13:53, Info.plist CFBundleShortVersionString=0.7.0, "
 "bundle id dev.adaptive.interaction.desktop); per "
 "docs/releases/evidence/2026-09-06-convergence/final/native-final-checkpoint/basic-clean/provenance.json its appSourceRef="
 "3b375bde4188e4a039852881d48d82ea9b619ce1, which is NOT an ancestor of HEAD 78dcda1 and NOT the v0.8.0 tag 1fa69b8 -> the App was NOT rebuilt from HEAD "
 "and its version string (0.7.0) lags the CLI (0.8.0). CLI target/debug/interact-ai sha256="
 "1e066d84d28b9397c6c6843522ec6a0fbaf692d10ffbea59481afb79860c84c7, `--version` = interact-ai 0.8.0, built from HEAD 2026-09-07 11:29. "
 "fake_iphone sha256=64ed2d995093064ed5db070705e7830448a11ae991f8d1823d6799429709127e (HEAD build). "
 "AX helper scripts/lib/tauri-ax.applescript sha256=3e1bd72d19bfebc876a8c69b47e72aa5634218fc371111dfe73e50d19e49817b.")

def env(agent, conn, dur, workdir, actual_model, cost="0 (no model API call; native UI / shell fixtures only)"):
    return {
        "sourceCommit": "78dcda1a3733c97d266ca9b60ad4461c69ca2032 (= origin/main; git log 1fa69b8..78dcda1 = 4 commits, all docs/test: 78dcda1, 3ba7364, b55c1be, 77e9fc4; diff touches only *.md/docs/releases/**/schemas + crates/interaction-adapter-declarative/tests/declarative_session_loop.rs). Working tree had 2 untracked paths created by other agents (docs/releases/phase-0-repository-state.md, scripts/tests/phase0/), so every driver recorded dirtyTree=true; this agent changed no repo file.",
        "binaryIdentity": BIN_ID,
        "agent": agent,
        "connectorVersion": conn,
        "requestedModel": "not-specifiable-via-gateway",
        "actualModel": actual_model,
        "actualModelSource": "provider-local-log (not via gateway)",
        "newOrResumed": "new",
        "workdir": workdir,
        "authorization": "Bearer human token read from the driver's isolated <temp home>/state/api-token; no consent granted, no emergency-stop unlock, no receptor/actuator enabled, INTERACT_AI_MOBILE_ADVERTISE=0.",
        "durationSeconds": dur,
        "costOrTokens": cost,
        "reasoning": "unknown (not applicable: no model invocation)",
    }

cases = []

cases.append({
 "id": "E0-01-native-preset-recovery",
 "scenario": "Native macOS app first-run setup -> change the companion preset (安靜) against a real daemon while an isolated loopback proxy injects 10 different faults -> kill/restart the app and daemon -> the durable preset marker must converge without overwriting a confirmed or newer config.",
 "agentOrSurface": "native Tauri app (interaction-control-center.app) driven by macOS Accessibility + production interact-ai daemon, isolated INTERACT_AI_HOME per case",
 "evidenceLevel": "native-desktop",
 "precondition": "macOS 26.2 (25C56) arm64, Accessibility permission already granted (probe: `osascript -e 'tell application \"System Events\" to get name of first process'` -> loginwindow, exit 0). App bundle and CLI present, unmodified during the run (driver re-hashes both at the end). No other interaction-desktop process running.",
 "steps": [
  "Verified HEAD vs tag: git log --oneline 1fa69b8..78dcda1 and git diff --stat 1fa69b8..78dcda1 -> docs/test only.",
  "Read scripts/tests/tauri-preset-recovery.py: per-case home = tempfile.mkdtemp('aip-native-preset-'), config/interaction.yaml written with apiHost 127.0.0.1 + the proxy port, env INTERACT_AI_HOME + INTERACT_AI_MOBILE_ADVERTISE=0 -> confirmed it never touches ~/.adaptive-interaction.",
  "Ran: python3 scripts/tests/tauri-preset-recovery.py --app <App> --out <R7>/preset (10 cases: refused, lost-reply, readback-failed, crash-before-runtime, crash-after-runtime, cleanup-failed, newer-choice, unrelated-prefs, future-marker, reconnect).",
  "For each case the driver launched a real daemon on a self-chosen ephemeral port, committed onboarding, launched the App, clicked the 安靜 checkbox through AX, injected the fault in the proxy, SIGKILLed the App (and for crash cases mid-write), restarted, and asserted the converged state.",
  "Collected result.json, per-case daemon logs, the recorded HTTP trace and AX attempts; removed the 10 isolated homes afterwards."
 ],
 "expected": {
  "ui": "After restart, the companion tab shows the 安靜 checkbox checked (AX value '1') for every fault case except newer-choice (where the user's newer choice wins) and future-marker (an unknown-format marker is kept and surfaced as 恢復標記, not silently dropped).",
  "coreState": "GET /v1/proactive-dialogue config.mode == 'necessary' (or 'off' for newer-choice); every other config field unchanged from the pre-fault snapshot; state/desktop.json companionPendingPresetOp cleared after recovery; unrelated prefs (companionOpacity) preserved.",
  "effect": "No extra conditional write is issued on restart for lost-reply / readback-failed / crash-after-runtime / cleanup-failed / newer-choice (asserted against the proxy's conditionalWrites counter); the App and daemon processes exit; no write outside the isolated home."
 },
 "result": "completed",
 "actual": "10/10 cases completed, exit 0, 113.66 s of driver-measured case time (117 s wall). Each case shows the converged effectiveConfig and prefs: mode='necessary' for 8 cases, 'off' for newer-choice, and future-marker kept companionPendingPresetOp={'format':99,'unknownIntent':'keep-me'} with config untouched (mode 'natural'). companionPendingPresetOp is null after recovery in the other 9 cases; unrelated-prefs kept companionOpacity 0.6, all others 0.8.",
 "evidence": [
  R+"/preset/result.json (10 objects under $.results, each status='completed'; $.results[*].effectiveConfig, $.results[*].prefs, $.results[*].requests (112-159 proxied HTTP calls per case), $.results[*].ax)",
  R+"/preset/result.json $.sourceSha=78dcda1a3733c97d266ca9b60ad4461c69ca2032, $.binarySha256=1103cda7..., $.cliSha256=1e066d84..., $.driverSha256=398ed57d..., $.humanOrHardwareEvidence=false",
  R+"/preset.stdout.txt (10 lines '<case> completed')",
  R+"/preset.cmd.txt (START=2026-09-07T09:08:30Z, EXIT=0, END=2026-09-07T09:10:27Z)",
  R+"/preset/refused.log line 1 'interact-ai daemon listening on http://127.0.0.1:57393' (each case picked its own ephemeral port: 58627 59321 60709 58964 61140 58201 59803 57762 57393 60275)",
  R+"/preset-homes.txt (the 10 isolated homes that were removed during cleanup)"
 ],
 "observations": [
  "The driver self-selects ephemeral ports via socket bind(0); it ignores the assigned 19070-19079 band. Ports actually used are listed above from the per-case daemon logs.",
  "future-marker is the honest-degradation case: an unknown marker format is preserved and surfaced in the UI (AX text 恢復標記) instead of being executed or dropped.",
  "This is the only one of the four native drivers that never types text and never opens a file panel - i.e. the only one that does not depend on synthetic keystroke delivery. It is also the only one that passed."
 ],
 "productDefects": [],
 "limitations": "Simulated faults only (an in-process loopback proxy), no human judgement, no hardware. The App under test was not rebuilt from HEAD (see binaryIdentity): it is the 2026-09-06 v0.8.0 candidate whose Info.plist still says 0.7.0, so this result binds to that binary, not to a HEAD build. dirtyTree=true in the driver provenance is caused by other agents' untracked files.",
 "cleanup": "Driver stopped its own App/daemon/proxy per case and deleted the api-token files; I then removed the 10 isolated homes listed in "+R+"/preset-homes.txt. `ps` shows no interaction-desktop process left. Repo unmodified.",
 "environment": env("none (native desktop UI; no AI agent session created)", "n/a (no agent connector exercised)", 117.0,
   "n/a (native UI only; runtime state confined to /var/folders/.../aip-native-preset-* isolated homes)",
   "unknown (no model was invoked; no provider session log exists for this case)")
})

cases.append({
 "id": "E0-11-native-settings-transfer",
 "scenario": "Native macOS export/import of companion settings through the real AX download and the real NSOpenPanel file picker: export -> re-import -> restart -> then four blocked imports (unknown character, unsupported version, malformed value, unwritable store) -> explicit blank name.",
 "agentOrSurface": "native Tauri app + production interact-ai daemon + real macOS Open panel, isolated INTERACT_AI_HOME",
 "evidenceLevel": "native-desktop",
 "precondition": "Same as E0-01. ~/Downloads had no pre-existing companion-settings*.json (the driver only deletes downloads it created, matched by dev/inode/sha256).",
 "steps": [
  "Ran: python3 scripts/tests/tauri-settings-walkthrough.py --app <App> --out <R7>/settings (first, preserved run).",
  "Diagnostic 1: identical command, --out <R7>/settings-diag1 (same long output path).",
  "Diagnostic 2: identical command with a short output path --out /private/tmp/aip-r7-s3, copied to <R7>/settings-diag2, to test whether the typed path length mattered.",
  "Wrote an instrumented probe (<R7>/probe_picker.py, in the run directory - no repo file was modified) that launches the same App/daemon, opens the same picker, and dumps the Open panel's window list and sheet before/after the AX choosefile step.",
  "probe1/probe2: import file inside the run output directory; probe3: same but schemaVersion 99; probe4: import file alone in a dedicated directory (/private/tmp/aip-r7-imp/one.json, companionName ZZTOP, expressiveness lively); probe5: same dedicated directory but with an invalid kind ('not-a-companion-settings'), which parseCompanionSettingsImport must reject.",
  "Decoded the App's remembered Open-panel directory: plutil -extract NSOSPLastRootDirectory raw -o - ~/Library/Preferences/dev.adaptive.interaction.desktop.plist | base64 -d | strings."
 ],
 "expected": {
  "ui": "'已匯出角色設定…' after export; '已匯入角色設定並套用。' after the valid import; 不是這台電腦認得的角色 / 不支援的版本 / 必須是開關值 / 無法確認設定已完整套用 for the four blocked imports.",
  "coreState": "state/desktop.json companionName becomes 還原名稱 and survives an App restart; prefs unchanged for each blocked import; unrelated companionOpacity stays 0.67; GET /v1/proactive-dialogue config unchanged by any import.",
  "effect": "A real companion-settings*.json file appears in ~/Downloads via the WebView download and is removed again at the end; the file chosen in the real Open panel is the file whose path was requested."
 },
 "result": "harness-failed",
 "actual": "Step 1 (export) completed and is genuinely verified: a 404-byte companion-settings JSON with sha256 f4a317120e88363c46cbe458760f1f684d698f65595420a04c528a31c8e3f944 appeared in ~/Downloads carrying the run's unique name, and was deleted again. Step 2 (re-import) never happened: the driver timed out waiting for companionName=='還原名稱' (error 'observable state did not arrive') in all three runs, so 7 of 8 steps - including all four correctly-blocked cases - are UNVERIFIED today. Instrumentation shows why, and it is not the product: the AX helper's `choosefile` never navigates. probe5 put a file with kind='not-a-companion-settings' alone in the target directory - parseCompanionSettingsImport would have to reject it - yet the App still showed '已匯入角色設定並套用。' and set companionName to ''. probe4 (only file = one.json, companionName ZZTOP, expressiveness lively) produced the same '' / 'natural' result. The applied values match, field for field, /private/tmp/adaptive-convergence-20260906/native-final-checkpoint/settings-legacy/blank-name.json (companionName '', companionExpressiveness 'natural', companionPack 'plain-text'), and NSOSPLastRootDirectory in the App's preferences decodes to exactly that 2026-09-06 directory. So the Open panel stays in its remembered directory and the driver's second Return opens the alphabetically first *.json there; the helper returns 'selected file in owned Open panel' unconditionally without verifying anything. The App behaved correctly for the file macOS actually handed it.",
 "evidence": [
  R+"/settings/result.json ($.error='observable state did not arrive'; $.steps[0]={'id':'export-file','status':'completed','bytes':404,'sha256':'f4a3171…'}; $.steps[1]={'id':'unfinished','status':'failed'})",
  R+"/settings/failed-ax.txt (status line still 'AXStaticText: 已匯出角色設定（不含權限、位置與歷史）。' and the nav/heading still show the pre-import name - the import handler never ran in the walkthrough)",
  R+"/settings-diag1/result.json and "+R+"/settings-diag2/result.json (same failure with a long and with a short output path -> not a path-length effect)",
  R+"/probe5/main-window-final.txt line 225 'AXStaticText: 已匯入角色設定並套用。' while /private/tmp/aip-r7-imp2/one.json had kind='not-a-companion-settings' (see "+R+"/probe5/valid-import.json copy of the same content) -> the App did not read the requested file",
  R+"/probe4/prefs-final.json and "+R+"/probe1/prefs-final.json ($.companionName='' , $.companionExpressiveness='natural', $.companionPack='plain-text')",
  R+"/probe1/panel-before-choosefile.txt ('WINDOW: Open subrole=AXDialog') vs "+R+"/probe1/panel-after-choosefile.txt (Open panel gone) - a file WAS chosen, just not ours",
  "/private/tmp/adaptive-convergence-20260906/native-final-checkpoint/settings-legacy/blank-name.json (companionName ''), and `plutil -extract NSOSPLastRootDirectory raw -o - ~/Library/Preferences/dev.adaptive.interaction.desktop.plist | base64 -d | strings` -> private / adaptive-convergence-20260906 / native-final-checkpoint / settings-legacy",
  REPO+"/scripts/lib/tauri-ax.applescript:51-63 (the choosefile handler: Cmd-Shift-G, fixed `delay 0.5`, keystroke of the whole path, Return, fixed `delay 1.2`, Return, then `return \"selected file in owned Open panel\"` with no verification)",
  REPO+"/apps/interaction-desktop/src/pages/character/CharacterLibrary.tsx:182-200 (doImportSettings) and src/companion/settingsTransfer.ts:110-132 (parse would throw on a wrong kind) - the basis for the probe5 discrimination"
 ],
 "observations": [
  "Harness defect (test code, not shipped product): scripts/lib/tauri-ax.applescript:51-63 reports success unconditionally and relies on two fixed delays; when the Go-to-Folder sheet does not receive the full typed path, the panel silently opens whatever file was pre-selected in its remembered directory. Repro: `python3 /private/tmp/.../R7/probe_picker.py <outdir> 19074` with IMPORT_KIND=not-a-companion-settings and IMPORT_PATH pointing at a lone file in a fresh directory -> UI still says '已匯入角色設定並套用。'.",
  "This failure mode can produce a FALSE PASS, not only a false failure: on 2026-09-06 the panel's remembered directory happened to be the run's own output directory, so the pre-selected file was frequently the intended one. The 2026-09-06 green result for this walkthrough should be treated as weaker evidence than it looks.",
  "~/Library/Preferences/dev.adaptive.interaction.desktop.plist mtime is still 2026-09-06 13:59 after all of today's runs, i.e. the panel never persisted a new root directory - consistent with 'navigation never succeeded'.",
  "The same underlying symptom (synthetic keystrokes not fully delivered into the App) also breaks E0-03; see that case for the direct truncation evidence.",
  "Export is genuinely verified end-to-end today: real WebView download into ~/Downloads, content whitelist assertion, and removal of only the file this run created."
 ],
 "productDefects": [],
 "limitations": "The import half of E0-11 (valid import, restart persistence, and the four correctly-blocked imports) is NOT verified today. I did not modify the harness to work around it (no repo writes allowed), so the block stands. Nothing here proves the product's import path is broken - probe evidence points the other way - but nothing here proves it works either.",
 "cleanup": "Each run deleted its own isolated home (shutil.rmtree in the driver) and its own ~/Downloads export (verified by dev/inode/sha256); ls ~/Downloads shows no companion-settings file left. I removed /private/tmp/aip-r7-s3, /private/tmp/aip-r7-imp and /private/tmp/aip-r7-imp2. Probe homes stay under "+R+"/probe*/home as evidence with their api tokens unlinked. No process left.",
 "environment": env("none (native desktop UI; no AI agent session created)", "n/a (no agent connector exercised)", 71.0,
   "n/a (native UI only; isolated /var/folders/.../aip-native-settings-* home, deleted by the driver)",
   "unknown (no model was invoked; no provider session log exists for this case)")
})

cases.append({
 "id": "E0-03-native-work-cancel-fixture",
 "scenario": "Submit read-only work from the real native composer to the repository fixture agents, then cancel: Codex turn cancellation via the native UI, and a long-running Claude fixture closed from the native UI with its process group verified dead.",
 "agentOrSurface": "native Tauri app + production interact-ai daemon + repository fixture agents (crates/interaction-runtime/tests/fixtures/fake_codex.sh, fake_claude.sh) - FIXTURE, not a real agent",
 "evidenceLevel": "native-desktop",
 "precondition": "Same as E0-01. The driver pins INTERACT_AI_CODEX_BIN / INTERACT_AI_CLAUDE_BIN to the two repo fixtures (deliberately overriding any inherited real-agent setting) and creates per-agent fixture workdirs inside its own temp home with a fake-mode file (turns / hang).",
 "steps": [
  "Read scripts/tests/tauri-work-cancel.py:95-140 to confirm fixture pinning, isolated home, self-chosen port, and that it never grants consent or allows write.",
  "Ran: python3 scripts/tests/tauri-work-cancel.py --app <App> --out <R7>/work.",
  "Driver launched daemon + App, went to the 工作 tab, and typed the task text 'Native fixture codex cancel' into the composer textarea via AX `fill` (click, Cmd-A, keystroke), then read the value back.",
  "Diagnostic re-run with --out <R7>/work-diag1 to test determinism."
 ],
 "expected": {
  "ui": "Composer holds the typed task text; the preview shows 不會修改 (read-only); after 開始 the work row appears; cancelling shows the cancelled/failed state honestly.",
  "coreState": "An agent session row reaching active, then cancelled (Codex turn/interrupt) and the Claude session closed; states read from the daemon, not from the UI text.",
  "effect": "The fixture's own process group receives turn/interrupt (fake-turn-interrupt file) and, for the Claude hang case, the fixture process group is gone after the native close."
 },
 "result": "harness-failed",
 "actual": "Both runs failed before any work was submitted, at the very first composer input: 'AX input readback: 幫你做什麼: timed out (expected state absent)' after 36.85 s / 37 s. The AX dumps show the textarea holding a TRUNCATED string - run 1 'Native fixture codex can', run 2 'Native fixture code' - against the expected 'Native fixture codex cancel'. Different truncation points on two consecutive runs, with every osascript call returning exit 0. So synthetic keystroke delivery into the App's WebView is lossy on this machine today; no session was ever created, and neither the Codex turn cancellation nor the Claude close was exercised. No product behaviour was observed at all beyond the composer rendering the characters it did receive.",
 "evidence": [
  R+"/work/result.json ($.result='failed', $.error='AX input readback: 幫你做什麼: timed out (expected state absent)', $.seconds=36.85, $.steps=[] (no step reached), $.artifactVerification.unchanged=true, $.realAgentExecuted=false, $.consentGranted=false, $.safetyUnlocked=false)",
  R+"/work/failed-ax.txt:40 'AXStaticText: Native fixture codex can' (truncated composer content)",
  R+"/work-diag1/failed-ax.txt:40 'AXStaticText: Native fixture code' (different truncation, same step)",
  R+"/work/result.json $.ax[-10:] - nine consecutive ['value','AXTextArea','幫你做什麼'] readbacks, all exit 0, none matching",
  R+"/work/processes.log ('interact-ai daemon listening on http://127.0.0.1:56823' + two benign macOS TSM/IMK log lines from the App)",
  R+"/work.cmd.txt (START=2026-09-07T09:28:23Z EXIT=1 END=2026-09-07T09:29:01Z) and "+R+"/work-diag1.cmd.txt (START=09:29:42Z EXIT=1 END=09:30:19Z)",
  REPO+"/scripts/tests/tauri-work-cancel.py:169-175 (fill = ax('fill',…) then wait for an exact value readback - the assertion that is timing out)"
 ],
 "observations": [
  "Harness/environment defect: AppleScript `keystroke` into the Tauri WebView drops characters non-deterministically on macOS 26.2 (25C56) on this machine, with load average ~3.3-4.2 from the other Phase-0 agents. Repro: `python3 scripts/tests/tauri-work-cancel.py --app <App> --out <fresh dir>` then read <dir>/failed-ax.txt line 40.",
  "Whether the loss happens in System Events' event posting or in the WebView/React controlled-input handling is NOT determined by this evidence; I did not modify the App or the helper to find out. Calling it a product input-handling defect would be unproven.",
  "The 2026-09-06 checkpoint ran the same driver (different sha - 897187c3 now vs the checkpoint's build) green in 52.9 s, so this is a today/this-machine regression in the AX layer, not a change in the App binary (identical sha256).",
  "Because the failure is before submission, nothing about consent, read-only scope, cancellation or process-group teardown was observed - those remain unverified today."
 ],
 "productDefects": [],
 "limitations": "Fixture agents only (no real Claude/Codex), by design of this driver. E0-03's actual subject matter (submit + cancel + verify process death) is entirely unverified today. Two runs, same step, different truncation.",
 "cleanup": "Driver terminated its own App and daemon and removed its temp home (no /var/folders/.../aip-native-work-* left). No fixture process survived (`ps` grep for fake_claude/fake_codex is empty). Repo unmodified.",
 "environment": env("repository fixture agents fake_codex.sh (sha256 b51ad9be5e1c90500ae2430b1d9af8104bf2734db83003fac15eb505443fe295) and fake_claude.sh (sha256 84816fd16d87229440db59c6d22fb6e6584ece65b41bcee9c585dbd1a40f9031) - FIXTURE; never actually launched because submission failed first",
   "fixture shell connectors pinned via INTERACT_AI_CODEX_BIN / INTERACT_AI_CLAUDE_BIN; no real claude-code or codex CLI involved", 38.0,
   "isolated fixture workdirs /var/folders/.../aip-native-work-*/{codex,claude}-fixture-work (created and deleted by the driver)",
   "unknown (no agent session was created, so no provider-side session log exists; fixtures are shell scripts with no model)")
})

cases.append({
 "id": "E0-08-native-mobile-fixture",
 "scenario": "Native macOS journeys against a simulated iPhone (committed fake_iphone binary): pair + capability negotiation, bidirectional semantic state, disconnect, reconnect, old-token rejection, re-pair, then four sensor-stop-unknown journeys.",
 "agentOrSurface": "native Tauri app + production interact-ai daemon + fake_iphone simulator (FIXTURE phone, pinned TLS/HMAC/wire) - no hardware",
 "evidenceLevel": "native-desktop",
 "precondition": "Same as E0-01, plus target/debug/examples/fake_iphone present (sha256 64ed2d99…, built from HEAD today 11:29). No consent, no receptor enabled, no real microphone capture (driver asserts all three false).",
 "steps": [
  "Read scripts/tests/tauri-mobile-sensor-walkthrough.py:160-190, 495-500 to confirm the isolated mkdtemp home, self-chosen port, INTERACT_AI_MOBILE_ADVERTISE=0 and that it drives the AX helper (no browser mock, no keystroke/file-picker use).",
  "Ran: python3 scripts/tests/tauri-mobile-sensor-walkthrough.py --app <App> --out <R7>/mobile.",
  "Diagnostic re-run: same command, --out <R7>/mobile-diag1."
 ],
 "expected": {
  "ui": "Each journey's native AX dump shows the matching connection/sensor text (已連線 / 未連線（能力不可用） / 移除此手機 / two-step remove confirmation / 停止結果未知).",
  "coreState": "Authoritative character-session snapshot revisions advance across the fixture-input -> runtime -> UI round trip; device ids stable across disconnect/reconnect; unresolvedStops retained as unknown after a daemon restart.",
  "effect": "The fake_iphone process actually pairs over TLS and receives host patches; stopping a sensor whose phone never answers stays 'unknown' rather than being claimed as stopped."
 },
 "result": "harness-failed",
 "actual": "Partial both times, then the same harness ceiling. Run 1: 4/10 journeys completed (mobile-pair-and-negotiate, mobile-bidirectional-semantic, mobile-disconnect, mobile-reconnect) in 312 s, then `osascript … dump` exceeded the driver's 55 s cap. Run 2: 6/10 completed (adds mobile-remove-old-token-rejected, mobile-pair-again) in 430 s, then the identical dump timeout. The AX full-window dump cost grows during a run: 12.8, 3.8, 7.1, 30.1, 30.1, 46.5, 37.4, 40.5 s, then >55 s. The four sensor journeys - the honesty-critical 'stop result unknown' ones - were never reached in either run. The journeys that did complete carry real core-state evidence (deviceId iphone-a823573d, snapshotRevision 4, revisionBefore 5 -> revisionAfter 6 with hostPatchRevision 6, result {'status':'applied'}).",
 "evidence": [
  R+"/mobile/result.json ($.result='failed', $.error=\"Command '[…tauri-ax.applescript, pid:36797, dump]' timed out after 55 seconds\", $.seconds=312.414, $.steps = 4 completed journeys, $.realMicrophoneCapture=false, $.consentGranted=false, $.safetyUnlocked=false, $.fakeIphoneSha256=64ed2d99…)",
  R+"/mobile-diag1/result.json ($.seconds=430.024, 6 completed journeys, same dump timeout on pid:41960; dump timings [12.832, 3.809, 7.114, 30.088, 30.105, 46.452, 37.389, 40.536])",
  R+"/mobile/{mobile-pair-and-negotiate,mobile-bidirectional-semantic,mobile-disconnect,mobile-reconnect}-ax.txt (native AX dumps per completed journey) and "+R+"/mobile/phone-1-events.json (fixture phone wire events)",
  R+"/mobile/processes.log ('interact-ai daemon listening on http://127.0.0.1:57362')",
  R+"/mobile.cmd.txt (START=2026-09-07T09:30:47Z EXIT=1 END=09:36:00Z), "+R+"/mobile-diag1.cmd.txt (START=09:36:37Z EXIT=1 END=09:43:47Z)",
  "docs/releases/evidence/2026-09-06-convergence/final/native-final-checkpoint/mobile-clean/result.json (2026-09-06 baseline: all 10 journeys, 334.4 s) - same App binary sha, so the difference is in the AX/host layer, not the App"
 ],
 "observations": [
  "Harness scalability limit: scripts/lib/tauri-ax.applescript's `dump` walks `entire contents` of the whole window through System Events; as the session accumulates device rows and history the traversal passes the driver's 55 s per-call cap (tauri-mobile-sensor-walkthrough.py:225 subprocess timeout). Repro: run the driver and watch $.ax dump timings grow monotonically.",
  "Second run got two journeys further than the first, so the cutoff point is load/timing dependent, but the degradation trend is systematic in both runs.",
  "The completed journeys are real native+fixture evidence and can be cited as such; the phone is a fixture process, never hardware (the ESP32/iPhone true-hardware acceptance remains at zero, unchanged).",
  "No product misbehaviour was observed in the 6 journeys that ran; the failure is entirely in the measuring apparatus."
 ],
 "productDefects": [],
 "limitations": "The four sensor journeys (sensor-stop-unknown, sensor-direct-remove-retains-unknown, sensor-unknown-after-daemon-restart, sensor-global-stop-separate-fixture) were not reached today - the part of E0-08 that proves 'stop result unknown' honesty is unverified in this baseline. Simulated iPhone only; no human evaluation, no real microphone.",
 "cleanup": "Driver removed its own isolated home (no /var/folders/.../aip-native-mobile-sensor-* left) and stopped App, daemon and fake_iphone; `ps` grep for fake_iphone/interaction-desktop is empty. Repo unmodified.",
 "environment": env("none (native desktop UI + fake_iphone fixture; no AI agent session created)",
   "fake_iphone example binary sha256 64ed2d995093064ed5db070705e7830448a11ae991f8d1823d6799429709127e (HEAD build 2026-09-07 11:29), pinned TLS/HMAC device wire protocol v1", 313.0,
   "n/a (native UI + fixture phone; isolated /var/folders/.../aip-native-mobile-sensor-* home, deleted by the driver)",
   "unknown (no model was invoked; no provider session log exists for this case)")
})

def cmd(name, c, s, e, sec, ex, out="", err=""):
    return {"name":name,"cmd":c,"cwd":REPO,"start":s,"end":e,"seconds":sec,"exit":ex,"stdout":out,"stderr":err}

commands = [
 cmd("git-log-tag-to-head","git log --oneline 1fa69b8..78dcda1 && git diff --stat 1fa69b8..78dcda1","2026-09-07T09:06:00Z","2026-09-07T09:06:02Z",2.0,0,
     "4 commits (78dcda1, 3ba7364, b55c1be, 77e9fc4); 276 files changed, 52962 insertions(+), 18 deletions(-), all docs/evidence/*.md plus crates/interaction-adapter-declarative/tests/declarative_session_loop.rs",""),
 cmd("ax-permission-probe","osascript -e 'tell application \"System Events\" to get name of first process'","2026-09-07T09:07:10Z","2026-09-07T09:07:11Z",1.0,0,"loginwindow",""),
 cmd("E0-01-preset","python3 scripts/tests/tauri-preset-recovery.py --app "+APP+" --out "+R+"/preset","2026-09-07T09:08:30Z","2026-09-07T09:10:27Z",117.0,0,"10 lines '<case> completed'",""),
 cmd("E0-11-settings-run1","python3 scripts/tests/tauri-settings-walkthrough.py --app "+APP+" --out "+R+"/settings","2026-09-07T09:11:04Z","2026-09-07T09:12:15Z",71.0,1,"export-file completed","(error recorded in result.json: 'observable state did not arrive')"),
 cmd("E0-11-settings-diag1","python3 scripts/tests/tauri-settings-walkthrough.py --app "+APP+" --out "+R+"/settings-diag1","2026-09-07T09:13:21Z","2026-09-07T09:14:43Z",82.0,1,"export-file completed","same failure"),
 cmd("E0-11-settings-diag2-shortpath","python3 scripts/tests/tauri-settings-walkthrough.py --app "+APP+" --out /private/tmp/aip-r7-s3","2026-09-07T09:14:53Z","2026-09-07T09:16:13Z",80.0,1,"export-file completed","same failure -> not path-length dependent"),
 cmd("probe1-picker-instrumented","python3 "+R+"/probe_picker.py "+R+"/probe1 19070","2026-09-07T09:18:22Z","2026-09-07T09:19:22Z",60.0,0,"imported: False name: ''",""),
 cmd("probe3-schema99","python3 "+R+"/probe_picker.py "+R+"/probe3 19072 SchemaProbe lively 99","2026-09-07T09:22:15Z","2026-09-07T09:23:15Z",60.0,0,"imported: False name: 'PROBE-START' (no import notice at all this run)",""),
 cmd("probe4-dedicated-dir","IMPORT_PATH=/private/tmp/aip-r7-imp/one.json python3 "+R+"/probe_picker.py "+R+"/probe4 19073 ZZTOP lively 1","2026-09-07T09:24:19Z","2026-09-07T09:25:16Z",57.0,0,"imported: False name: '' expr: natural (UI said 已匯入角色設定並套用。)",""),
 cmd("probe5-invalid-kind","IMPORT_KIND=not-a-companion-settings IMPORT_PATH=/private/tmp/aip-r7-imp2/one.json python3 "+R+"/probe_picker.py "+R+"/probe5 19074 KINDPROBE lively 1","2026-09-07T09:25:48Z","2026-09-07T09:26:46Z",58.0,0,"UI said 已匯入角色設定並套用。 for a file the parser must reject -> the panel opened a different file",""),
 cmd("decode-open-panel-bookmark","plutil -extract NSOSPLastRootDirectory raw -o - ~/Library/Preferences/dev.adaptive.interaction.desktop.plist | base64 -d | strings","2026-09-07T09:27:30Z","2026-09-07T09:27:31Z",1.0,0,"private / adaptive-convergence-20260906 / native-final-checkpoint / settings-legacy",""),
 cmd("E0-03-work-run1","python3 scripts/tests/tauri-work-cancel.py --app "+APP+" --out "+R+"/work","2026-09-07T09:28:23Z","2026-09-07T09:29:01Z",38.0,1,"failed: AX input readback: 幫你做什麼: timed out (expected state absent)",""),
 cmd("E0-03-work-diag1","python3 scripts/tests/tauri-work-cancel.py --app "+APP+" --out "+R+"/work-diag1","2026-09-07T09:29:42Z","2026-09-07T09:30:19Z",37.0,1,"same failure; composer truncated to 'Native fixture code'",""),
 cmd("E0-08-mobile-run1","python3 scripts/tests/tauri-mobile-sensor-walkthrough.py --app "+APP+" --out "+R+"/mobile","2026-09-07T09:30:47Z","2026-09-07T09:36:00Z",313.0,1,"4 journeys completed, then osascript dump timed out after 55 s",""),
 cmd("E0-08-mobile-diag1","python3 scripts/tests/tauri-mobile-sensor-walkthrough.py --app "+APP+" --out "+R+"/mobile-diag1","2026-09-07T09:36:37Z","2026-09-07T09:43:47Z",430.0,1,"6 journeys completed, then the same 55 s dump timeout",""),
 cmd("cleanup-verify","ps -axo pid,pgid,ppid,etime,stat,command | grep -iE 'interaction-desktop|fake_iphone|fake_claude|fake_codex'; ls ~/Downloads | grep -i companion; git status --porcelain","2026-09-07T09:45:10Z","2026-09-07T09:45:12Z",2.0,1,"no matching processes; no companion-settings in ~/Downloads; only other agents' untracked paths in git status",""),
]

doc = {
 "group": "R7 native-desktop E2E (Phase 0 baseline) - repo's own real Tauri walkthrough drivers, run one at a time on the exclusive native UI slot",
 "cases": cases,
 "commands": commands,
 "openQuestions": [
  "Is the synthetic-keystroke loss (E0-03) in System Events / macOS 26.2 event posting, or in the App's WebView/React controlled inputs? Deciding this needs either a non-WebView control typed by the same helper, or App instrumentation - both were out of scope (no repo writes, no rebuild).",
  "Why did scripts/lib/tauri-ax.applescript's Go-to-Folder navigation work on 2026-09-06 and not today with an identical App binary and helper sha? Candidates: system load, macOS focus timing, or the panel's remembered directory coincidentally matching the target on 2026-09-06.",
  "How much of the 2026-09-06 native-final-checkpoint 'green' for settings-clean/settings-legacy is load-bearing, given that a stale remembered picker directory can make choosefile pass for the wrong reason? Re-running those with a per-step assertion on the chosen file name would settle it.",
  "Does the AX dump cost grow because the DOM grows during a mobile run, or because of machine load? A run on an idle machine, or a dump scoped to a subtree, would separate the two.",
  "Should the App bundle be rebuilt from HEAD before the next native baseline? Today's App is the 2026-09-06 candidate (appSourceRef 3b375bde, Info.plist 0.7.0) and is not reproducible from HEAD 78dcda1 - disk pressure (12 GB free) blocked a rebuild here."
 ],
 "notes": "Only E0-01 passed. E0-11, E0-03 and E0-08 are classified harness-failed, not product-failed: in each case the failure is in the AX measuring apparatus (unverified file-panel selection; lossy synthetic keystrokes; AX tree traversal exceeding the driver's own timeout), and in every instrumented probe the App did the right thing for the input it actually received. Nothing in this run demonstrates a product defect, and nothing in it verifies the import, work-cancel or sensor-unknown behaviours either - they are simply unmeasured today. No repo file was modified, no cargo/pnpm build was run, ~/.adaptive-interaction, ~/.claude and ~/.codex were never written, and every process and temp home created here was cleaned up by pid/path."
}

p = pathlib.Path(R+"/result-R7.json")
p.write_text(json.dumps(doc, ensure_ascii=False, indent=2)+"\n")
print("wrote", p, p.stat().st_size, "bytes;", len(doc["cases"]), "cases;", len(doc["commands"]), "commands")
