#!/usr/bin/env python3
import json

R6 = "/private/tmp/claude-501/-Users-user-Workspace-claude-lab-adaptive-interaction/95db6375-b13e-4494-9180-4b15da263657/scratchpad/e2e2/runs/R6"

env_common = {
    "sourceCommit": "78dcda1a3733c97d266ca9b60ad4461c69ca2032",
    "binaryIdentity": "interact-ai 0.8.0 sha256:1e066d84d28b9397c6c6843522ec6a0fbaf692d10ffbea59481afb79860c84c7 (built 2026-09-07 11:29 from HEAD, per task spec; not rebuilt this session)",
    "agent": "none (no claude-code/codex agent session; CLI+HTTP+fixture WS only)",
    "connectorVersion": "n/a",
    "requestedModel": "not-specifiable-via-gateway",
    "actualModel": "n/a (no agent session involved in E0-08)",
    "actualModelSource": "n/a",
    "reasoning": "n/a",
    "newOrResumed": "new (fresh isolated INTERACT_AI_HOME each daemon start; state persisted across the two mid-run daemon restarts)",
    "workdir": R6,
    "authorization": "human control token (home/state/api-token) via CLI --config/env and curl Authorization: Bearer; no agent-scope token used",
    "costOrTokens": "unknown (no LLM agent session; CLI/HTTP/WS calls only, no token-metered API involved)",
}

def env(duration, extra=None):
    e = dict(env_common)
    e["durationSeconds"] = duration
    if extra:
        e.update(extra)
    return e

cases = []

# 1. pair
cases.append({
    "id": "E0-08-fixture-pair",
    "scenario": "iPhone fixture pairing: interact-ai mobile pair issues a code/fingerprint/port; fake_iphone performs the TOFU-pinned TLS handshake + HMAC pair-challenge/pair-response; daemon records the device as paired and connected.",
    "agentOrSurface": "CLI (interact-ai mobile pair, mobile status) + HTTP API (/v1/mobile/status) + fixture binary (fake_iphone)",
    "evidenceLevel": "fixture",
    "precondition": "Isolated daemon running on 127.0.0.1:19060 (INTERACT_AI_HOME=R6/home, INTERACT_AI_MOBILE_ADVERTISE=0); mobile provider not yet started (GET /v1/mobile/status showed started:false, port:null before first `mobile pair`).",
    "steps": [
        "interact-ai mobile pair --json -> {code, fingerprint, port} (pair-2.json)",
        "Launch target/debug/examples/fake_iphone --port <port> --fingerprint <fp> --code <code> --name '模擬 iPhone（fixture）', stdin fed via a persistent anonymous-pipe command pump (tail -f cmds.txt | fake_iphone) so stdin outlives any single Bash tool-call process",
        "fake_iphone stdout: {deviceId,deviceToken} then {event:status,micLevel:false} then {event:connected} (phone.log lines 1-3)",
        "interact-ai mobile status --json -> device iphone-696703d5 connected:true with model/name/pairedAt/permissions/sensors (daemon-responses.jsonl label mobile-status-after-pair2-connected)",
    ],
    "expected": {
        "ui": "mobile status (CLI/API) lists the device as connected",
        "coreState": "daemon's paired-device record has this deviceId with a live WS session; fingerprint/port match the pairing session issued",
        "effect": "a real TLS 1.3 WebSocket connection is established to the daemon's mobile listener, authenticated via the daemon-issued device token from the pair-response handshake",
    },
    "result": "completed",
    "evidence": [
        f"{R6}/logs/pair-2.json",
        f"{R6}/logs/phone.log:1-3",
        f"{R6}/logs/daemon-responses.jsonl (label=mobile-status-after-pair2-connected)",
    ],
    "observations": [
        "A first pairing attempt (pair-1.json, device iphone-579732d7) was lost to a harness bug on my side: I initially fed fake_iphone's stdin via `exec 9>fifo` inside the same transient Bash tool-call shell that launched it; when that shell process exited at the end of the tool call, the writer fd closed, fake_iphone saw EOF on stdin and exited. This left a stale never-reconnected device record (iphone-579732d7) visible in later mobile-status output. Fixed for all subsequent steps by using `tail -f cmds.txt | fake_iphone` (anonymous pipe inside one self-contained backgrounded subshell) so the writer end never depends on a transient shell's lifetime. This is a testing-harness lesson on my part, not a product defect.",
        "Plain `cmd > namedfifo &` in this zsh environment fails immediately (ENXIO, no error surfaced) unless a reader is already blocked on the fifo when the write-open happens; documenting for any future agent hitting the same pattern.",
    ],
    "productDefects": [],
    "limitations": "Fixture (fake_iphone), not a real iPhone; TOFU pin/HMAC pairing crypto exercised, but iOS app UI/permission prompts are not.",
    "cleanup": "Device iphone-696703d5 was later revoked (see E0-08-fixture-revoke); daemon killed at end of run. Stale iphone-579732d7 record remains harmless in the isolated home's mobile-devices.json (never reconnected).",
    "environment": env(3, {"durationSeconds": 3}),
})

# 2. bidirectional
cases.append({
    "id": "E0-08-fixture-bidirectional",
    "scenario": "Device-reported status (micLevel, permissions) flows into daemon status/activeSensors; daemon-issued commands (stop-all, AIP capability/behavior-intent) flow to the device and are acknowledged/echoed back — covering both directions of the mobile channel.",
    "agentOrSurface": "CLI (mobile status, receptors) + HTTP API (/v1/status, /v1/receptors/{id}, /v1/onboarding/*) + fixture (fake_iphone stdin ops: status, aip-capability, aip-touch)",
    "evidenceLevel": "fixture",
    "precondition": "Device iphone-696703d5 paired and connected (see E0-08-fixture-pair).",
    "steps": [
        "fake_iphone stdin <- {op:status,micLevel:true,permissions:{microphone:granted,location:denied,bluetooth:notDetermined}} (phone.log line 4)",
        "GET /v1/mobile/status -> device.sensors.micLevel=true, device.permissions matches exactly what was sent",
        "GET /v1/status -> activeSensors=[] even though the phone reports micLevel:true, because the iphone.mic-level receptor is disabled by default (matches CLAUDE.md invariant: mic/camera receptors default off)",
        "POST /v1/onboarding/preview {enableReceptors:[iphone.mic-level]} -> refused with consent_required (bulk onboarding path categorically refuses consent-gated receptors, by design per crates/interaction-runtime/src/human.rs:729)",
        "interact-ai session start; interact-ai session consent receptor:iphone.mic-level; POST /v1/onboarding/commit -> still refused (bulk path never accepts consent-gated components even once consent is granted, by design)",
        "PATCH /v1/receptors/iphone.mic-level {enabled:true} -> {enabled:true} (the correct explicit per-component enable path)",
        "GET /v1/status -> activeSensors now shows iphone.mic-level, state=active, startedBy=iphone:iphone-696703d5 — device-reported state now honestly visible daemon-wide (tray/UI-facing field)",
    ],
    "expected": {
        "ui": "device-reported sensor/permission state is visible in mobile status immediately; it only promotes to the global activeSensors (tray/UI) list once the corresponding receptor is enabled, honoring the default-off invariant for sensitive receptors",
        "coreState": "runtime registry receptor-enabled flag AND live device self-report both gate activeSensors membership (crates/interaction-runtime/src/mobile.rs:3095 mobile_scoped_captures requires both)",
        "effect": "no external side effect (dry observation channel); the mic-level receptor is consent-gated and only became visible after the explicit PATCH enable path, not the bulk onboarding wizard path",
    },
    "result": "completed",
    "evidence": [
        f"{R6}/logs/phone.log:4",
        f"{R6}/logs/daemon-responses.jsonl (labels: status-micLevel-and-permissions-after-fixture-status-op, v1-status-activeSensors-empty-before-receptor-enable, receptor-iphone.mic-level-consent-gated-refusal, onboarding-commit-still-refuses-bulk-enable-even-after-consent, receptor-patch-enable-succeeds-explicit-path, v1-status-activeSensors-after-enable-and-fixture-micLevel-true)",
            "crates/interaction-runtime/src/mobile.rs:3080-3120 (mobile_active_sensors/mobile_scoped_captures gating logic, read to explain the activeSensors delay)",
            "crates/interaction-runtime/src/human.rs:729-742 (consent-gated bulk-onboarding refusal, read to explain the onboarding/commit refusal)",
        ],
    "observations": [
        "The task brief assumed device-reported micLevel:true would show up in /v1/status.activeSensors right away; it does not until the receptor is explicitly enabled. This is correct-by-design (CLAUDE.md: '麥克風...預設關閉'), not a defect — documented here so the discrepancy from the task's expectation is explicit rather than silently glossed over.",
    ],
    "productDefects": [],
    "limitations": "Only the mic-level sensor channel and one AIP touch round-trip were exercised as 'bidirectional'; other mobile actuator wire types (haptic/notification/etc.) were not exercised in this run.",
    "cleanup": "Consent/receptor-enable state is per-daemon-process and does not persist across restart (confirmed later in E0-08-fixture-core-offline), so no lingering state beyond this isolated home.",
    "environment": env(70),
})

# 3. stop-unresolved
cases.append({
    "id": "E0-08-fixture-stop-unresolved",
    "scenario": "Desktop asks one paired iPhone to stop sensing; the fixture (by default) does not auto-ack, so the outcome is honestly 'unknown', not 'stopped', both in the immediate CLI response and in status.activeSensors; if the device also disconnects before acking, the stop is promoted into the durable /v1/sensors/unresolved inbox (human-review-only, never a stop confirmation) until acked or dismissed.",
    "agentOrSurface": "CLI (interact-ai mobile stop-sensors) + HTTP API (/v1/status, /v1/sensors/unresolved, /v1/sensors/unresolved/{id}/dismiss) + fixture (fake_iphone, run without --auto-ack-stop-all)",
    "evidenceLevel": "fixture",
    "precondition": "Part A: device iphone-696703d5 connected with iphone.mic-level receptor enabled and streaming (micLevel:true) (E0-08-fixture-bidirectional). Part B: a fresh device iphone-0164ded7 was paired on a restarted daemon instance specifically to drive the device-disconnects-while-unacked path.",
    "steps": [
        "Part A: interact-ai mobile stop-sensors iphone-696703d5 --json -> {outcome:unknown, waitedMs:2002} after the fixture's 2s STOP_SENSORS_WAIT budget elapses with no ack (fixture receives {event:stop-all,sensors:true}, phone.log line 5, and deliberately does not reply)",
        "Part A: GET /v1/status -> activeSensors[0].state=stop-unknown, purpose text '停止結果未知（iPhone 未回覆，可能仍在擷取）' — the sensor is NOT silently dropped from the live list (CLAUDE.md '感測不靜默')",
        "Part A: GET /v1/sensors/unresolved -> still empty while the device remains connected (the unresolved inbox is for stops that left the *live* list, not merely unacked-but-still-visible ones)",
        "Part A: fake_iphone stdin <- {op:ack-stop-all} (phone.log line 6) -> GET /v1/status activeSensors=[] (state honestly cleared once acked)",
        "Part B (fresh daemon+device to exercise the promotion path): stop-sensors iphone-0164ded7 (unacked) then fake_iphone stdin <- {op:disconnect} ~0.3s later (phone3.log lines 4-6)",
        "Part B: GET /v1/sensors/unresolved -> now non-empty: one entry sourceId=provider.mobile, reason=stop-all-sensors, sensors=[iphone.mic-level], confirmedStopped:false, unresolvedStopHealth.recoveryUnknown:true",
        "Part B: POST /v1/sensors/unresolved/provider.mobile/dismiss {generation:2097154} -> {confirmedStopped:false, dismissed:true, note:'human reviewed and dismissed the reminder; no source stop confirmation'} — dismiss never upgrades to a stop confirmation",
    ],
    "expected": {
        "ui": "an unacked stop never silently reads as 'stopped'; while the device stays connected it shows 'stop-unknown' live in activeSensors, and once the device disappears while still unacked it also appears in the durable unresolved-stops inbox for human review",
        "coreState": "honest ladder is enforced at the data model level: SensorStopStatus never becomes Stopped/AlreadyStopped without an actual device ack; unresolvedStopHealth.recoveryUnknown flips true",
        "effect": "CLI/API never claims the microphone actually stopped capturing; only a human dismiss (explicitly non-claiming) or a genuine device ack can clear the record",
    },
    "result": "completed",
    "evidence": [
        f"{R6}/logs/phone.log:5-6",
        f"{R6}/logs/phone3.log:4-6",
        f"{R6}/logs/daemon-responses.jsonl (labels: mobile-stop-sensors-cli-no-ack-outcome-unknown, v1-sensors-unresolved-empty-while-still-connected-stop-unknown, v1-status-activeSensors-stop-unknown-state, v1-status-activeSensors-cleared-after-fixture-ack-stop-all, orphan-unresolved-populated-immediately-on-disconnect-while-stop-pending, unresolved-dismiss-does-not-claim-stopped)",
        "crates/interaction-runtime/src/mobile.rs:68 (STOP_SENSORS_WAIT=2s)",
        "crates/interaction-runtime/src/sensor_source.rs:36-44 (ORPHAN_CAPTURE_VISIBLE design comment)",
    ],
    "observations": [
        "/v1/sensors/unresolved populated immediately on disconnect-while-pending rather than only after the documented 60s ORPHAN_CAPTURE_VISIBLE window — the window appears to govern how long a removed-but-possibly-still-capturing source stays visible in the live activeSensors list after removal, not the delay before the unresolved inbox entry itself appears. I did not trace the exact code path separating these two timers further; noting the empirical timing here rather than asserting the mechanism from code alone.",
    ],
    "productDefects": [],
    "limitations": "Did not exercise the case of a device that reconnects and genuinely re-confirms a stop after being in the unresolved inbox (only disconnect->unresolved->dismiss was driven end-to-end).",
    "cleanup": "Part B device iphone-0164ded7 quit cleanly; unresolved record dismissed; second temporary daemon instance (for Part B) killed at end.",
    "environment": env(95),
})

# 4. aip
cases.append({
    "id": "E0-08-fixture-aip",
    "scenario": "AIP 1.0 Character Session negotiation and interaction: capability negotiation registers the device as a session member; a touch event advances the authoritative session revision, dispatches a Behavior Intent to the device, and returns an 'applied' result; a duplicate messageId is deduplicated without re-applying or advancing revision.",
    "agentOrSurface": "CLI (interact-ai character session status/diagnostics) + fixture (fake_iphone stdin ops: aip-capability, aip-touch)",
    "evidenceLevel": "fixture",
    "precondition": "Device iphone-696703d5 connected (E0-08-fixture-pair); baseline character session at revision=1, sessionEpoch=1 (empty members list).",
    "steps": [
        "fake_iphone stdin <- {op:aip-capability} (phone.log lines 7-9): daemon replies capability(negotiated, role=remote-renderer, intents include react-happily-to-touch:exact) then a state snapshot with the device now listed as an online member; revision 1->2",
        "interact-ai character session status --json -> revision=2 confirmed via CLI (independent of the fixture's own view)",
        "fake_iphone stdin <- {op:aip-touch,kind:tap,messageId:e2e-touch-1} (phone.log lines 10-13): daemon emits a state patch (mood->happy, activity->reacting, revision->3), a character.behavior.request command (intent=react-happily-to-touch, haptic hint) targeted at the device, and a result{status:applied} correlated to e2e-touch-1",
        "interact-ai character session status --json -> revision=3, sequence=6",
        "interact-ai character session diagnostics --json -> member iphone-696703d5 presence=online, identityStrength=paired-token, counters{applied:1,accepted:1,intents.emitted:1}",
    ],
    "expected": {
        "ui": "character session status/diagnostics reflect the negotiated member and the interaction outcome",
        "coreState": "revision/sequence monotonically advance only on genuinely applied state changes; capability negotiation and touch-result correlation are both visible via causationId/correlationId",
        "effect": "a Behavior Intent (character.behavior.request) is actually dispatched over the WS connection to the device — verified by the fixture receiving it, not just by a runtime log claiming it was sent",
    },
    "result": "completed",
    "evidence": [
        f"{R6}/logs/phone.log:7-13",
        f"{R6}/logs/daemon-responses.jsonl (label=character-session-diagnostics-after-touch-applied)",
    ],
    "observations": [
        "Did not separately drive the documented duplicate-messageId dedup path (already covered by scripts/v03-cli-e2e.sh's existing assertion suite per repo evidence; not re-verified live in this run to keep the case within budget) or the unknown-message-type honest-rejection path (aip-raw with an unsupported messageType).",
    ],
    "productDefects": [],
    "limitations": "Only one intent (react-happily-to-touch) and one input kind (tap) were exercised; 'settle' is negotiated as unsupported by this fixture and was not driven.",
    "cleanup": "Session state carried forward into the disconnect/reconnect/resume case below; no separate cleanup needed.",
    "environment": env(15),
})

# 5. disconnect-reconnect
cases.append({
    "id": "E0-08-fixture-disconnect-reconnect",
    "scenario": "Device disconnects mid-session; character-session presence honestly shows 'reconnecting' (not 'offline') while the device is gone, members are preserved; device reconnects and resumes with a bounded set of missed patches instead of a full snapshot, continuing the same revision/sequence/epoch lineage.",
    "agentOrSurface": "CLI (interact-ai character session status/diagnostics/resume) + HTTP API (/v1/mobile/status) + fixture (fake_iphone stdin ops: disconnect, reconnect, aip-capability, aip-resume)",
    "evidenceLevel": "fixture",
    "precondition": "Device iphone-696703d5 connected, character session at revision=4 (post-touch, E0-08-fixture-aip); pre-disconnect revision/epoch captured as REV0=4, EPOCH=1.",
    "steps": [
        "fake_iphone stdin <- {op:disconnect} (phone.log line 15, event=disconnected reason=requested)",
        "GET /v1/mobile/status -> device.connected=false (no explicit lastSeen field on the device entry itself; see observations)",
        "interact-ai character session diagnostics --json -> member presence=reconnecting (not offline), lastSeenAt preserved — matches the documented contract (offline only after a full session timeout, not on first disconnect)",
        "fake_iphone stdin <- {op:reconnect} (phone.log lines 16-17): fixture reconnects the WS with its existing device token, receives {event:connected}",
        "fake_iphone stdin <- {op:aip-capability} (phone.log lines 19-22): re-negotiates; member presence flips back to online, revision 5->6",
        "fake_iphone stdin <- {op:aip-resume,lastRevision:4,lastSequence:0,epoch:1} (phone.log lines 23-24): daemon replies response{kind:patches} with exactly the two patches (revision 5 'reconnecting', revision 6 'online') the device missed while disconnected — not a full snapshot",
        "interact-ai character session resume --last-revision 4 --epoch 1 --json (CLI path, independent of the fixture's own resume call) -> same kind:patches result",
    ],
    "expected": {
        "ui": "presence transitions disconnected->reconnecting->online are visible without the member disappearing",
        "coreState": "resume returns exactly the missed revisions (patches), continuing the same sessionEpoch, not a stale or duplicated state",
        "effect": "the device's own view (via aip-resume over its live WS) and the CLI's independent view (character session resume) agree on the same patch set",
    },
    "result": "completed",
    "evidence": [
        f"{R6}/logs/phone.log:15-24",
        f"{R6}/logs/daemon-responses.jsonl (label=diagnostics-presence-reconnecting-immediately-after-disconnect)",
    ],
    "observations": [
        "/v1/mobile/status device entries carry only `pairedAt`, no `lastSeen`/`lastSeenAt` field — the task brief's step description assumed a lastSeen field would appear there on disconnect. The actual last-seen signal lives on the character-session diagnostics member entry (lastSeenAt), not on the mobile status device entry. Documented as a discrepancy from the task's assumption, not treated as a defect (no invariant requires it there).",
    ],
    "productDefects": [],
    "limitations": "Only a clean, quick (~sub-second) disconnect/reconnect was exercised; did not drive the full session-timeout-to-offline transition (would require waiting past the session timeout window, out of budget for this run).",
    "cleanup": "Device left connected; subsequently revoked in the next case.",
    "environment": env(25),
})

# 6. revoke
cases.append({
    "id": "E0-08-fixture-revoke",
    "scenario": "Human revokes a paired device via DELETE /v1/mobile/devices/{id} (interact-ai mobile revoke); the live connection is closed immediately with an honest auth-fail(revoked) frame, the device record disappears from mobile status, and a subsequent reconnect attempt with the now-invalid token is rejected.",
    "agentOrSurface": "CLI (interact-ai mobile revoke, mobile status) + fixture (fake_iphone stdin op: reconnect)",
    "evidenceLevel": "fixture",
    "precondition": "Device iphone-696703d5 connected and online (E0-08-fixture-disconnect-reconnect).",
    "steps": [
        "interact-ai mobile revoke iphone-696703d5 --json -> {revoked:iphone-696703d5, wasConnected:true}",
        "fake_iphone log within ~1.5s: {event:auth-fail,reason:revoked} then {event:disconnected,reason:'IO error: peer closed connection...'} (phone.log lines 25-26) — matches the <2s immediate-disconnect contract documented in mobile_loop.rs (revoke_disconnects_live_connection_immediately)",
        "interact-ai mobile status --json -> device iphone-696703d5 no longer present in the devices list at all (revoke removes the record, not just marks it disconnected)",
        "fake_iphone stdin <- {op:reconnect} using its now-stale token (phone.log line 27) -> {event:auth-fail,reason:'unknown device or bad token (possibly revoked)'} — honest rejection, no partial/degraded access",
    ],
    "expected": {
        "ui": "device disappears from mobile status immediately after revoke",
        "coreState": "the device token is invalidated server-side; not merely disconnected but unable to re-authenticate",
        "effect": "the live socket is torn down fast (well under any human-perceptible delay) and any further connection attempt with the old token is refused",
    },
    "result": "completed",
    "evidence": [
        f"{R6}/logs/phone.log:25-27",
        f"{R6}/logs/daemon-responses.jsonl (labels: mobile-revoke-response, mobile-status-after-revoke-device-removed)",
    ],
    "observations": [],
    "productDefects": [],
    "limitations": "None beyond fixture-vs-real-device (see E0-08-real-iphone).",
    "cleanup": "Device fully removed by the revoke itself; fixture process quit cleanly afterward.",
    "environment": env(6),
})

# 7. core-offline
cases.append({
    "id": "E0-08-fixture-core-offline",
    "scenario": "F-06: the daemon (core) goes offline while an iPhone is connected, and a previously-paired device's token is expected to still work once the daemon restarts with the same state directory.",
    "agentOrSurface": "shell (SIGKILL on the daemon PID) + fixture (fake_iphone) + a hand-written ad-hoc Python WS client (ws_auth_probe.py) used as a substitute after finding the shipped fixture cannot survive this scenario (see productDefects)",
    "evidenceLevel": "fixture",
    "precondition": "A second, fresh device (iphone-8842a4be) paired and connected specifically for this scenario, so the revoke from the previous case cannot confound the result.",
    "steps": [
        "kill -KILL <daemon pid> -> daemon confirmed gone from `ps`",
        "fake_iphone's live socket dies immediately as a side effect of the daemon process exiting: {event:disconnected,reason:'IO error: peer closed connection...'} (phone2.log line 4) — this part needed no explicit reconnect op, the OS itself tore the socket down",
        "Sent {op:reconnect} to the still-alive fake_iphone process anyway: it attempted a fresh TCP connect, got ECONNREFUSED, printed 'fake_iphone: 連不上 wss://127.0.0.1:18790/：IO error: Connection refused (os error 61)' to stderr and then EXITED THE WHOLE PROCESS (see productDefects) instead of emitting a recoverable event and staying alive for a later retry",
        "Restarted the daemon with the same INTERACT_AI_HOME: log shows 'reclaiming stale instance lock pid=89643'; GET /v1/mobile/status shows both previously-paired devices still present (state persisted across the kill)",
        "Since the shipped fixture had already exited and offers no CLI way to resume a specific existing device token in a new process, wrote a minimal ad-hoc Python client (ws_auth_probe.py, TLS cert verification disabled since this only probes protocol-level auth, not the TOFU pinning already covered by E0-08-fixture-pair) performing exactly the same {type:auth,deviceId,token} handshake fake_iphone's own reconnect() does",
        "Ran the probe against the still-dead daemon first: ConnectionRefusedError (probe-while-dead.json) — confirms the client-side failure mode is a plain refused connection, honestly surfaced",
        "Ran the same probe again after the daemon restart, same deviceId+token: {type:auth-ok} (probe-after-restart.json) — the daemon accepted the previously-issued token for a previously-paired device after a full process restart",
    ],
    "expected": {
        "ui": "while core is down, any client honestly reports connection failure rather than a false 'connected'/'stopped'/'success'; once core is back, a previously-paired device's token continues to work without re-pairing",
        "coreState": "paired-device records (including tokens) persist to disk and survive a daemon restart",
        "effect": "a real WS TLS handshake + auth frame round-trip succeeds post-restart with the pre-restart token",
    },
    "result": "completed",
    "evidence": [
        f"{R6}/logs/phone2.log:1-4",
        f"{R6}/logs/phone2.stderr.log",
        f"{R6}/logs/daemon-restart.log",
        f"{R6}/logs/probe-while-dead.json",
        f"{R6}/logs/probe-after-restart.json",
        f"{R6}/logs/ws_auth_probe.py",
        "crates/interaction-runtime/examples/fake_iphone.rs:159-177,297-322 (connect()/reconnect() — root cause of the fixture defect below)",
    ],
    "observations": [
        "The daemon-side half of F-06 (persisted device token survives a core restart) is verified with genuine protocol-level evidence (a real TLS WS handshake against the real daemon), not simulated. The device-side half (does the real fixture binary gracefully survive this and recover) is NOT fully verified, because of the productDefect below — I built a minimal substitute client instead of modifying the fixture, per the hard rule against editing repo files.",
    ],
    "productDefects": [
        {
            "title": "fake_iphone.rs reconnect() calls a connect() helper that hard-exits the whole process (die() -> process::exit(2)) on ANY connection failure, including an ordinary 'core is currently offline' condition, instead of returning an error the caller can report and recover from",
            "severity": "medium",
            "location": "crates/interaction-runtime/examples/fake_iphone.rs:159-177 (fn connect, the Err arm at line 177 calls die()), invoked from fn reconnect at crates/interaction-runtime/examples/fake_iphone.rs:298",
            "repro": "Pair a device with fake_iphone against a running daemon, then `kill -KILL` the daemon while fake_iphone stays alive, then send it {\"op\":\"reconnect\"} on stdin. Expected (per the fixture's own doc comment and reconnect()'s own auth-fail/reconnect-failed emit arms) is a recoverable event so a test harness can retry {op:reconnect} later once the daemon is back up. Actual: stderr prints 'fake_iphone: 連不上 wss://...：IO error: Connection refused (os error 61)' and the process exits(2) immediately, so no further stdin commands (including a later successful reconnect once the daemon restarts) can ever be delivered to that process instance.",
            "evidence": [
                f"{R6}/logs/phone2.stderr.log",
                f"{R6}/logs/phone2.log:4 (last line; process confirmed gone from ps immediately after)",
                "crates/interaction-runtime/examples/fake_iphone.rs:177",
                "crates/interaction-runtime/examples/fake_iphone.rs:298",
            ],
            "phase0MinimalFixCandidate": True,
        }
    ],
    "limitations": "Because of the fixture defect above, the exact scenario the task described ('fake 的行為 reconnect 失敗... 重啟後同一個 process 用同一個 token 重連') could not be observed end-to-end through the shipped fixture binary itself; it was verified at the protocol level with a substitute client instead. The daemon-side persistence claim is solid; the fixture's own resilience to this scenario is not (and is now a filed defect).",
    "cleanup": "Second daemon instance from this scenario was itself later killed/restarted for the stop-unresolved Part B deep-dive, then finally killed at the very end of the run; fake_iphone (phone2) had already self-exited.",
    "environment": env(45),
})

# 8. real iphone
cases.append({
    "id": "E0-08-real-iphone",
    "scenario": "Read-only environment check for a real iPhone acceptance run: is a device connected/paired, is Developer Mode on, and is a codesigning Team configured for apps/interaction-ios?",
    "agentOrSurface": "shell (xcrun devicectl, security find-identity) + apps/interaction-ios/scripts/device-build.sh --check-only (read-only gate script, verified by reading its source before running: WORK_DIR is a mktemp dir removed by an EXIT trap, and --check-only returns before any xcodebuild/devicectl install/launch call)",
    "evidenceLevel": "not-run",
    "precondition": "DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer (Xcode 26.6, confirmed present, not just Command Line Tools); a real iPhone 11 physically connected via USB.",
    "steps": [
        "xcrun devicectl list devices -> one iOS device: 'Alex', iPhone 11 (iPhone12,1), state 'available (paired)'",
        "security find-identity -v -p codesigning -> one valid identity in the keychain: 'Apple Development: Feng-Chou Lee (PMFJ9X99V7)'",
        "apps/interaction-ios/scripts/device-build.sh --check-only: step 0-2 pass (tools present, device found, Developer Mode=enabled); step 3/5 (簽章 Team) FAILS: 找不到任何簽章 Team ID — reads Xcode's own IDEProvisioningTeams defaults plist (not the system keychain) and finds no entry",
    ],
    "expected": {
        "ui": "device-build.sh either passes all preflight gates (ready for a real device build+install+launch acceptance run) or names exactly what a human must do",
        "coreState": "n/a (read-only check, no daemon/session involved)",
        "effect": "no side effects: no xcodebuild, no devicectl install/launch was invoked (confirmed by reading device-build.sh's --check-only branch before running it: it exits at step 3/5, before step 4/5 which is the first step that touches xcodebuild)",
    },
    "result": "needs-environment",
    "evidence": [
        "terminal output of `xcrun devicectl list devices` (this session's transcript; device 'Alex' iPhone 11 iPhone12,1)",
        "terminal output of `security find-identity -v -p codesigning` (this session's transcript)",
        "terminal output of `apps/interaction-ios/scripts/device-build.sh --check-only` (this session's transcript; failed at step 3/5 with the Team-ID gate message)",
        "apps/interaction-ios/scripts/device-build.sh:158-190 (the Team-ID gate itself, read to confirm --check-only performs no writes/builds)",
    ],
    "observations": [
        "A codesigning identity DOES exist in the system keychain (Apple Development: Feng-Chou Lee), which differs from the prior session's memory note ('Xcode 帳號 Team 缺失') — but device-build.sh's gate specifically reads Xcode's own IDEProvisioningTeams account-list preference (via `defaults export com.apple.dt.Xcode`), which is empty, i.e. no Apple ID is signed into Xcode's Settings -> Accounts even though a certificate exists in the keychain from some earlier signing operation. These are two different stores; the script is correct to gate on the one xcodebuild -allowProvisioningUpdates actually needs.",
    ],
    "productDefects": [],
    "limitations": "No attempt was made to sign in to Xcode, build, install, or launch on the real device, per the task's explicit instruction to do read-only checks only for this case.",
    "cleanup": "None (read-only).",
    "environment": {
        **env_common,
        "durationSeconds": 20,
        "agent": "n/a (local Xcode/devicectl tooling, no daemon involved)",
        "newOrResumed": "n/a",
        "authorization": "local user session (no daemon token involved)",
        "workdir": "/Users/user/Workspace/claude-lab/adaptive-interaction/apps/interaction-ios/scripts",
    },
})

commands = [
    {"name":"setup-home","cmd":"mkdir -p R6/home/config R6/home/state; cat > R6/home/config/interaction.yaml <<< 'apiHost: 127.0.0.1\\napiPort: 19060'","start":"2026-09-07T09:03:00Z","end":"2026-09-07T09:03:01Z","seconds":1,"exit":0},
    {"name":"daemon-start-1","cmd":"env -i HOME=$HOME PATH=$PATH INTERACT_AI_HOME=R6/home INTERACT_AI_MOBILE_ADVERTISE=0 interact-ai serve > R6/logs/daemon.log 2>&1 &","start":"2026-09-07T09:03:43Z","end":"2026-09-07T09:03:45Z","seconds":2,"exit":0,"stdout":"interact-ai daemon listening on http://127.0.0.1:19060"},
    {"name":"mobile-status-before-pair","cmd":"interact-ai mobile status --json","start":"2026-09-07T09:03:48Z","end":"2026-09-07T09:03:48Z","seconds":0,"exit":0,"stdout":"{\"started\":false,\"port\":null,\"devices\":[]}"},
    {"name":"mobile-pair-1-aborted","cmd":"interact-ai mobile pair --json","start":"2026-09-07T09:03:52Z","end":"2026-09-07T09:03:52Z","seconds":0,"exit":0},
    {"name":"fake-iphone-1-fifo-attempt-lost-to-harness-bug","cmd":"fake_iphone --port 18790 --fingerprint <fp1> --code <code1> (stdin via exec 9>fifo in a transient shell)","start":"2026-09-07T09:03:52Z","end":"2026-09-07T09:04:04Z","seconds":12,"exit":0,"stderr":"process exited when the transient shell holding the fifo writer fd closed (my harness bug, see E0-08-fixture-pair observations)"},
    {"name":"mobile-pair-2-working","cmd":"interact-ai mobile pair --json","start":"2026-09-07T09:08:17Z","end":"2026-09-07T09:08:17Z","seconds":0,"exit":0},
    {"name":"fake-iphone-2-anon-pipe-pump","cmd":"( tail -n +1 -f phone.cmds.txt | fake_iphone --port 18790 --fingerprint <fp2> --code <code2> --name '模擬 iPhone（fixture）' ) > phone.log 2> phone.stderr.log &","start":"2026-09-07T09:08:18Z","end":"2026-09-07T09:08:20Z","seconds":2,"exit":0,"stdout":"{\"deviceId\":\"iphone-696703d5\",...}\\n{\"event\":\"connected\"}"},
    {"name":"mobile-status-connected","cmd":"interact-ai mobile status --json","start":"2026-09-07T09:08:20Z","end":"2026-09-07T09:08:20Z","seconds":0,"exit":0},
    {"name":"send-fixture-status-micLevel-true","cmd":"printf status-op >> phone.cmds.txt","start":"2026-09-07T09:08:38Z","end":"2026-09-07T09:08:40Z","seconds":2,"exit":0},
    {"name":"v1-status-activeSensors-check-1","cmd":"curl /v1/status","start":"2026-09-07T09:08:58Z","end":"2026-09-07T09:08:58Z","seconds":0,"exit":0,"stdout":"activeSensors=[]"},
    {"name":"onboarding-preview-refused","cmd":"curl -X POST /v1/onboarding/preview -d '{\"enableReceptors\":[\"iphone.mic-level\"]}'","start":"2026-09-07T09:09:10Z","end":"2026-09-07T09:09:10Z","seconds":0,"exit":0,"stdout":"{\"error\":{\"code\":\"consent_required\"}}"},
    {"name":"session-start","cmd":"interact-ai session start --json","start":"2026-09-07T09:09:25Z","end":"2026-09-07T09:09:25Z","seconds":0,"exit":0},
    {"name":"session-consent-mic-level","cmd":"interact-ai session consent receptor:iphone.mic-level --json","start":"2026-09-07T09:09:25Z","end":"2026-09-07T09:09:25Z","seconds":0,"exit":0},
    {"name":"onboarding-commit-still-refused","cmd":"curl -X POST /v1/onboarding/commit -d '{\"enableReceptors\":[\"iphone.mic-level\"]}'","start":"2026-09-07T09:09:25Z","end":"2026-09-07T09:09:25Z","seconds":0,"exit":0,"stdout":"{\"error\":{\"code\":\"consent_required\"}}"},
    {"name":"receptor-patch-enable","cmd":"curl -X PATCH /v1/receptors/iphone.mic-level -d '{\"enabled\":true}'","start":"2026-09-07T09:09:42Z","end":"2026-09-07T09:09:42Z","seconds":0,"exit":0,"stdout":"{\"enabled\":true}"},
    {"name":"v1-status-activeSensors-check-2","cmd":"curl /v1/status","start":"2026-09-07T09:09:45Z","end":"2026-09-07T09:09:45Z","seconds":0,"exit":0,"stdout":"activeSensors=[{kind:iphone.mic-level,state:active}]"},
    {"name":"mobile-stop-sensors-cli","cmd":"interact-ai mobile stop-sensors iphone-696703d5 --json","start":"2026-09-07T09:10:08Z","end":"2026-09-07T09:10:10Z","seconds":2,"exit":0,"stdout":"{\"outcome\":\"unknown\",\"waitedMs\":2002}"},
    {"name":"v1-sensors-unresolved-check-1","cmd":"curl /v1/sensors/unresolved","start":"2026-09-07T09:10:13Z","end":"2026-09-07T09:10:13Z","seconds":0,"exit":0,"stdout":"{\"unresolvedStops\":[]}"},
    {"name":"fixture-ack-stop-all","cmd":"printf ack-stop-all-op >> phone.cmds.txt","start":"2026-09-07T09:10:44Z","end":"2026-09-07T09:10:45Z","seconds":1,"exit":0},
    {"name":"aip-capability-1","cmd":"printf aip-capability-op >> phone.cmds.txt","start":"2026-09-07T09:11:24Z","end":"2026-09-07T09:11:26Z","seconds":2,"exit":0},
    {"name":"aip-touch-1","cmd":"printf aip-touch-op >> phone.cmds.txt","start":"2026-09-07T09:11:31Z","end":"2026-09-07T09:11:33Z","seconds":2,"exit":0},
    {"name":"character-session-diagnostics-1","cmd":"interact-ai character session diagnostics --json","start":"2026-09-07T09:11:33Z","end":"2026-09-07T09:11:33Z","seconds":0,"exit":0},
    {"name":"fixture-disconnect-1","cmd":"printf disconnect-op >> phone.cmds.txt","start":"2026-09-07T09:11:42Z","end":"2026-09-07T09:11:43Z","seconds":1,"exit":0},
    {"name":"fixture-reconnect-1","cmd":"printf reconnect-op >> phone.cmds.txt","start":"2026-09-07T09:12:04Z","end":"2026-09-07T09:12:06Z","seconds":2,"exit":0},
    {"name":"aip-capability-2","cmd":"printf aip-capability-op >> phone.cmds.txt","start":"2026-09-07T09:12:06Z","end":"2026-09-07T09:12:08Z","seconds":2,"exit":0},
    {"name":"aip-resume-1","cmd":"printf aip-resume-op >> phone.cmds.txt","start":"2026-09-07T09:12:07Z","end":"2026-09-07T09:12:09Z","seconds":2,"exit":0},
    {"name":"cli-character-session-resume","cmd":"interact-ai character session resume --last-revision 4 --epoch 1 --json","start":"2026-09-07T09:12:43Z","end":"2026-09-07T09:12:43Z","seconds":0,"exit":0},
    {"name":"mobile-revoke","cmd":"interact-ai mobile revoke iphone-696703d5 --json","start":"2026-09-07T09:12:44Z","end":"2026-09-07T09:12:44Z","seconds":0,"exit":0,"stdout":"{\"revoked\":\"iphone-696703d5\",\"wasConnected\":true}"},
    {"name":"fixture-reconnect-after-revoke","cmd":"printf reconnect-op >> phone.cmds.txt","start":"2026-09-07T09:12:45Z","end":"2026-09-07T09:12:47Z","seconds":2,"exit":0,"stdout":"{\"event\":\"auth-fail\",\"reason\":\"unknown device or bad token (possibly revoked)\"}"},
    {"name":"fixture-1-quit-cleanup","cmd":"printf quit-op >> phone.cmds.txt; kill leftover tail pump","start":"2026-09-07T09:12:52Z","end":"2026-09-07T09:12:53Z","seconds":1,"exit":0},
    {"name":"mobile-pair-3-for-core-offline","cmd":"interact-ai mobile pair --json","start":"2026-09-07T09:12:45Z","end":"2026-09-07T09:12:45Z","seconds":0,"exit":0},
    {"name":"fake-iphone-2-launch","cmd":"( tail -n +1 -f phone2.cmds.txt | fake_iphone --port 18790 ... --name '模擬 iPhone（fixture 2）' ) > phone2.log 2>phone2.stderr.log &","start":"2026-09-07T09:12:46Z","end":"2026-09-07T09:12:48Z","seconds":2,"exit":0},
    {"name":"kill-daemon-sigkill","cmd":"kill -KILL 89643","start":"2026-09-07T09:14:37Z","end":"2026-09-07T09:14:38Z","seconds":1,"exit":0},
    {"name":"fixture-reconnect-after-kill-dies","cmd":"printf reconnect-op >> phone2.cmds.txt","start":"2026-09-07T09:14:38Z","end":"2026-09-07T09:14:41Z","seconds":3,"exit":0,"stderr":"fake_iphone: 連不上 wss://127.0.0.1:18790/：IO error: Connection refused (os error 61)"},
    {"name":"ws-probe-while-dead","cmd":"python3 ws_auth_probe.py 18790 iphone-8842a4be <token>","start":"2026-09-07T09:14:42Z","end":"2026-09-07T09:14:45Z","seconds":3,"exit":0,"stdout":"{\"outcome\":\"error\",\"error\":\"ConnectionRefusedError...\"}"},
    {"name":"daemon-restart-1","cmd":"env -i HOME=$HOME PATH=$PATH INTERACT_AI_HOME=R6/home INTERACT_AI_MOBILE_ADVERTISE=0 interact-ai serve > R6/logs/daemon-restart.log 2>&1 &","start":"2026-09-07T09:14:47Z","end":"2026-09-07T09:14:49Z","seconds":2,"exit":0,"stdout":"reclaiming stale instance lock pid=89643"},
    {"name":"ws-probe-after-restart","cmd":"python3 ws_auth_probe.py 18790 iphone-8842a4be <token>","start":"2026-09-07T09:14:57Z","end":"2026-09-07T09:15:00Z","seconds":3,"exit":0,"stdout":"{\"outcome\":\"reply\",\"reply\":{\"type\":\"auth-ok\"}}"},
    {"name":"kill-daemon-2","cmd":"kill 67267","start":"2026-09-07T09:15:20Z","end":"2026-09-07T09:15:21Z","seconds":1,"exit":0},
    {"name":"devicectl-list-devices","cmd":"DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun devicectl list devices","start":"2026-09-07T09:15:30Z","end":"2026-09-07T09:15:33Z","seconds":3,"exit":0,"stdout":"Alex  iPhone 11 (iPhone12,1)  available (paired)"},
    {"name":"security-find-identity","cmd":"security find-identity -v -p codesigning","start":"2026-09-07T09:15:33Z","end":"2026-09-07T09:15:34Z","seconds":1,"exit":0,"stdout":"1) B4D44B78... \\\"Apple Development: Feng-Chou Lee (PMFJ9X99V7)\\\""},
    {"name":"device-build-check-only","cmd":"apps/interaction-ios/scripts/device-build.sh --check-only","start":"2026-09-07T09:15:35Z","end":"2026-09-07T09:15:50Z","seconds":15,"exit":1,"stdout":"steps 0-2 pass, Developer Mode enabled","stderr":"[閘門未通過] 找不到任何簽章 Team ID"},
    {"name":"daemon-restart-2-for-orphan-test","cmd":"env -i HOME=$HOME PATH=$PATH INTERACT_AI_HOME=R6/home INTERACT_AI_MOBILE_ADVERTISE=0 interact-ai serve > R6/logs/daemon-restart2.log 2>&1 &","start":"2026-09-07T09:17:17Z","end":"2026-09-07T09:17:19Z","seconds":2,"exit":0,"stdout":"reclaiming stale instance lock pid=67267"},
    {"name":"receptor-check-not-persisted","cmd":"curl /v1/receptors/iphone.mic-level","start":"2026-09-07T09:17:19Z","end":"2026-09-07T09:17:19Z","seconds":0,"exit":0,"stdout":"{\"manifest\":{\"availability\":\"disabled\"}}"},
    {"name":"re-enable-consent-and-receptor","cmd":"session start; session consent receptor:iphone.mic-level; curl -X PATCH /v1/receptors/iphone.mic-level -d '{\"enabled\":true}'","start":"2026-09-07T09:17:20Z","end":"2026-09-07T09:17:22Z","seconds":2,"exit":0},
    {"name":"mobile-pair-4-orphan-test","cmd":"interact-ai mobile pair --json","start":"2026-09-07T09:17:28Z","end":"2026-09-07T09:17:28Z","seconds":0,"exit":0},
    {"name":"fake-iphone-3-launch","cmd":"( tail -n +1 -f phone3.cmds.txt | fake_iphone --port 18790 ... --name '模擬 iPhone（fixture 3）' ) > phone3.log 2>phone3.stderr.log &","start":"2026-09-07T09:17:29Z","end":"2026-09-07T09:17:31Z","seconds":2,"exit":0},
    {"name":"fixture3-status-micLevel-true","cmd":"printf status-op >> phone3.cmds.txt","start":"2026-09-07T09:17:38Z","end":"2026-09-07T09:17:39Z","seconds":1,"exit":0},
    {"name":"stop-sensors-then-disconnect-while-pending","cmd":"interact-ai mobile stop-sensors iphone-0164ded7 --json; printf disconnect-op >> phone3.cmds.txt","start":"2026-09-07T09:17:39Z","end":"2026-09-07T09:17:41Z","seconds":2,"exit":0,"stdout":"{\"outcome\":\"unknown\"}"},
    {"name":"v1-sensors-unresolved-check-2-populated","cmd":"curl /v1/status; curl /v1/sensors/unresolved","start":"2026-09-07T09:17:41Z","end":"2026-09-07T09:17:41Z","seconds":0,"exit":0,"stdout":"unresolvedStops has 1 entry, sourceId=provider.mobile"},
    {"name":"dismiss-unresolved-stop","cmd":"curl -X POST /v1/sensors/unresolved/provider.mobile/dismiss -d '{\"generation\":2097154}'","start":"2026-09-07T09:17:45Z","end":"2026-09-07T09:17:45Z","seconds":0,"exit":0,"stdout":"{\"confirmedStopped\":false,\"dismissed\":true}"},
    {"name":"fixture3-quit-and-final-cleanup","cmd":"printf quit-op >> phone3.cmds.txt; kill leftover tail pumps (49059,49060,89506,89507); kill daemon 88097","start":"2026-09-07T09:17:46Z","end":"2026-09-07T09:18:10Z","seconds":24,"exit":0},
    {"name":"final-process-sanity-check","cmd":"ps -axo pid,ppid,stat,command | grep -E 'fake_iphone|phone.?cmds|interact-ai serve'","start":"2026-09-07T09:18:36Z","end":"2026-09-07T09:18:37Z","seconds":1,"exit":0,"stdout":"(empty — no lingering fixture/daemon processes owned by this run)"},
]

result = {
    "group": "E0-08",
    "cases": cases,
    "commands": commands,
    "openQuestions": [
        "What actual mechanism separates ORPHAN_CAPTURE_VISIBLE (60s, documented as the live-list visibility window) from the near-immediate population of /v1/sensors/unresolved observed on disconnect-while-pending? Not traced further in this run.",
        "Would the CLAUDE.md-documented 'high risk capabilities do not auto-recover after restart' invariant (confirmed here for the mic-level receptor-enabled flag) also cover a currently-streaming device across a *live* daemon restart (rather than the enable-flag alone, tested cold)? Not exercised.",
    ],
    "notes": "All work done against an isolated daemon on 127.0.0.1:19060 (INTERACT_AI_HOME under this run's out dir), never against ~/.adaptive-interaction. No repo files were modified or built; the one product-level finding (fake_iphone.rs's connect()/reconnect() die()-on-any-failure bug) is reported, not fixed, per the hard rule. Wall-clock time for this run was longer than the sum of case durationSeconds because of two things outside the product's own latency: (1) an early self-inflicted harness bug (fifo-writer-lifetime vs. Bash tool-call process lifetime) that cost several minutes to diagnose and replace with an anonymous-pipe pump architecture, documented in E0-08-fixture-pair's observations so a future agent doesn't repeat it; (2) a deliberate extra daemon restart cycle to get live (not just static-code-read) evidence for the /v1/sensors/unresolved promotion path in E0-08-fixture-stop-unresolved. Case durationSeconds report the actual operation-level latency, not this session's total wall time.",
}

with open(f"{R6}/result-R6.json", "w", encoding="utf-8") as f:
    json.dump(result, f, ensure_ascii=False, indent=2)

print("wrote", f"{R6}/result-R6.json")
print("cases:", len(cases), "commands:", len(commands))
