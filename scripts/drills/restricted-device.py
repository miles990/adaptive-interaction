#!/usr/bin/env python3
"""Clean-checkout N2/N5 restricted-device drill. Uses production adapter + pty simulator."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

root = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser()
parser.add_argument("--evidence-dir", required=True, type=Path)
args = parser.parse_args()
args.evidence_dir.mkdir(parents=True, exist_ok=True)
env = dict(os.environ, CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="4")
commands = [
    ["python3", "scripts/tests/aip-applied-receiver.py"],
    *[["cargo", "test", "-p", "interaction-runtime", "--test", "declarative_session_loop", test, "--", "--exact"] for test in [
        "negotiated_state_applied_tracks_snapshot_patch_loss_rebind_and_stale_receipts",
        "legacy_fragmenting_devices_remain_unconfirmed_after_all_writes_succeed",
        "a_device_without_fragmentation_degrades_to_intent_only",
        "an_event_only_device_is_an_event_source_member",
    ]],
]
report = dict(evidenceLevel="production-adapter+pty-simulator", sourceSha=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
    dirty=bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=root, text=True)),
    contract="device wire v1.3 optional aip.applied/1; legacy v1.x preserved; AIP 1.0 unchanged",
    changeReason="Reusable spec templates and production paths replace unpublished scratch branches; no core transport/device special case is added.",
    productionPath="Runtime.register_declarative_spec -> DeviceBinding -> DeviceLink -> SerialLink -> pty receiver -> bound receipt -> CharacterSession diagnostics",
    dataRetention="Each run uses only its own temporary Runtime and provider IDs; no user preference, consent or package directory is loaded or rewritten.",
    modules=["examples/device-profiles/state-applied-serial.template.json", "DeviceLink", "DeviceBinding", "StateAppliedTracker"],
    humanDecisions=["Use a simulator, not a board", "Simulator explicitly opts into receipt support; firmware does not"],
    background=["AIP capability versus progress", "Semantic schema/hash and receipt context", "639-byte UTF-8 serial frame bound"],
    recovery="Each fixture owns its Runtime tempdir/provider ID/pty process; disable/rebind preserves user choices and uses a new transfer context.",
    results=[])
for index, command in enumerate(commands):
    start = time.monotonic()
    result = subprocess.run(command, cwd=root, env=env, capture_output=True, text=True)
    path = args.evidence_dir / f"{index+1}.log"
    path.write_text(result.stdout + result.stderr)
    report["results"].append(dict(command=command, seconds=round(time.monotonic()-start, 3), exitCode=result.returncode, log=str(path)))
    (args.evidence_dir / "restricted-device.json").write_text(json.dumps(report, ensure_ascii=False, indent=2)+"\n")
    print(f"{index+1}/{len(commands)}: exit={result.returncode} seconds={report['results'][-1]['seconds']}", flush=True)
    if result.returncode: raise SystemExit(result.returncode)
