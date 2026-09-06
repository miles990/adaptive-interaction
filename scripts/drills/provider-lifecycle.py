#!/usr/bin/env python3
"""N5 lifecycle drill: real CLI/application paths with a production pty adapter.

Only standard Python libraries; fixture spec and simulator are committed. No
consent is granted and no emergency stop is cleared. This is simulator evidence.
"""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("port", nargs="?", type=int, default=0)
    parser.add_argument("--output-dir", type=Path)
    args = parser.parse_args()
    cli = Path(os.environ.get("INTERACT_AI_BIN", ROOT / "target/debug/interact-ai")).resolve()
    if not cli.is_file():
        parser.error("build the current checkout first: cargo build -p interaction-cli")
    output = (args.output_dir or Path(tempfile.mkdtemp(prefix="provider-drill-evidence-"))).resolve()
    output.mkdir(parents=True, exist_ok=True)
    if (output / "result.json").exists():
        parser.error("output directory already contains result.json; choose a fresh directory")
    started = time.monotonic()
    steps = []
    result = {
        "drill": "provider-disable-reenable", "evidenceLevel": "production-cli-pty-simulator",
        "sourceSha": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "dirtyTree": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()),
        "binarySha256": hashlib.sha256(cli.read_bytes()).hexdigest(),
        "binaryVersion": subprocess.check_output([str(cli), "--version"], text=True).strip(),
        "steps": steps,
        "productionPath": "CLI -> HTTP -> Runtime::transition_provider/revoke_provider -> run_declarative_rebind -> production serial adapter",
        "contracts": ["docs/aip/device-profile.md#61", "docs/aip/privacy.md#51"],
        "decisions": ["preserve the manually disabled non-consent receptor", "no consent or safety override", "Removed remains experimental"],
        "backgroundKnowledge": ["provider lifecycle", "source/binding generation", "simulator versus real-board evidence"],
        "modifiedModules": ["temporary config/adapters/button-box.yaml from the committed template"],
        "status": "failed",
    }
    processes = []

    def stop(proc):
        if proc and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=12)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=3)

    try:
        with contextlib.ExitStack() as stack:
            work = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix="provider-drill-")))
            home = work / "home"
            adapters = home / "config/adapters"
            adapters.mkdir(parents=True)
            pty_file = work / "pty"
            if args.port:
                port = args.port
            else:
                with socket.socket() as sock:
                    sock.bind(("127.0.0.1", 0))
                    port = sock.getsockname()[1]
            env = dict(os.environ, INTERACT_AI_HOME=str(home), INTERACT_AI_API=f"http://127.0.0.1:{port}", INTERACT_AI_MOBILE_ADVERTISE="0")
            # Closing the picker socket does not reserve the port. The daemon
            # must remain alive and accept this isolated home's token below.
            with (output / "simulator.log").open("w") as log:
                sim = subprocess.Popen(["python3", str(ROOT / "scripts/esp32-serial-sim.py"), "--device-id", "button-box-01", "--pairing-code", "9927", "--pty-path-file", str(pty_file), "--no-frag"], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
            processes.append(sim)
            stack.callback(stop, sim)

            def wait_for(label, predicate, timeout=35):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    if predicate():
                        return
                    time.sleep(0.15)
                raise AssertionError(f"{label}: timed out after {timeout}s")

            wait_for("simulator startup", lambda: pty_file.is_file() and bool(pty_file.read_text().strip()))
            template = (ROOT / "examples/adapters/event-source-button.yaml").read_text()
            (adapters / "button-box.yaml").write_text(template.replace("/dev/cu.usbmodem-CHANGE-ME", pty_file.read_text().strip()))

            def call(*command, allow_failure=False):
                reply = subprocess.run([str(cli), "--json", *command], env=env, cwd=ROOT, text=True, capture_output=True, timeout=12)
                if reply.returncode and not allow_failure:
                    raise AssertionError(f"CLI {command[0]} failed with exit {reply.returncode}")
                try:
                    value = json.loads(reply.stdout)
                except json.JSONDecodeError:
                    value = None
                return value, reply.returncode

            def launch():
                with (output / "daemon.log").open("a") as log:
                    daemon = subprocess.Popen([str(cli), "serve", "--port", str(port)], env=env, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
                processes.append(daemon)
                stack.callback(stop, daemon)
                def ready():
                    if daemon.poll() is not None:
                        raise AssertionError("isolated daemon exited before ready; inspect daemon.log")
                    return call("status", allow_failure=True)[1] == 0
                wait_for("daemon authenticated startup", ready)
                return daemon

            provider = "provider.adapter.button-box"
            def state():
                return call("providers", "show", provider)[0]["state"]
            def audit():
                rows = call("audit", "--limit", "200")[0]
                (output / "audit.json").write_text(json.dumps(rows, ensure_ascii=False, indent=2))
                return rows
            def rebound_rows():
                return [row for row in audit() if row["kind"] == "provider.rebound" and row["detail"].get("providerId") == provider]
            def availability(receptor):
                return call("receptors", "inspect", receptor)[0]["manifest"]["availability"]
            def note(name, at):
                steps.append({"name": name, "status": "completed", "elapsedSeconds": round(time.monotonic() - at, 3)})

            daemon = launch()
            stage = time.monotonic()
            call("providers", "transition", provider, "--state", "available")
            wait_for("initial handshake", lambda: state() == "available")
            assert availability("button-box.button") == "available"
            other_before = availability("system.time")
            call("receptors", "disable", "button-box.button")
            assert availability("button-box.button") == "disabled"
            note("initial handshake and human OFF choice", stage)
            generations = []
            for cycle in range(2):
                stage = time.monotonic()
                call("providers", "transition", provider, "--state", "disabled")
                assert state() == "disabled"
                assert availability("system.time") == other_before, "disabling A changed B"
                disabled_rows = [row for row in audit() if row["kind"] == "provider.transitioned" and row["detail"].get("providerId") == provider and row["detail"].get("state") == "disabled"]
                assert disabled_rows and disabled_rows[0]["detail"]["closedLinks"], "disable did not record closing the old transport"
                completed_before = len(rebound_rows())
                reply, _ = call("providers", "transition", provider, "--state", "available")
                assert reply["state"] == "disconnected", "Available must not precede rebind completion"
                wait_for("rebind handshake and audit", lambda: state() == "available" and len(rebound_rows()) > completed_before)
                assert availability("button-box.button") == "disabled", "rebind erased the human OFF choice"
                record = max(rebound_rows(), key=lambda row: row["detail"]["generation"])["detail"]
                assert record["handshake"] == "ready"
                assert record["drained"] is True, "old session dispatch was not drained"
                assert "button-box.button" in record["keptDisabledReceptors"]
                generations.append(record["generation"])
                note(f"disable/re-enable cycle {cycle + 1}", stage)
            assert generations[1] > generations[0], "rebind reused a generation"
            result["bindingGenerations"] = generations
            stage = time.monotonic()
            call("providers", "revoke", provider)
            assert state() == "revoked"
            call("providers", "transition", provider, "--state", "available", allow_failure=True)
            assert state() == "revoked", "revocation was resurrected"
            assert availability("system.time") == other_before
            stop(daemon)
            daemon = launch()
            assert state() == "revoked", "restart resurrected revoked provider"
            (output / "provider-after-restart.json").write_text(json.dumps(call("providers", "show", provider)[0], ensure_ascii=False, indent=2))
            (output / "audit.json").write_text(json.dumps(audit(), ensure_ascii=False, indent=2))
            note("revoke rejects re-enable and survives process restart", stage)
            result["status"] = "completed"
    except Exception as error:
        result["error"] = str(error)
        raise
    finally:
        for proc in reversed(processes):
            stop(proc)
        result["elapsedSeconds"] = round(time.monotonic() - started, 3)
        (output / "result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
        print(json.dumps({"status": result["status"], "evidence": str(output), "elapsedSeconds": result["elapsedSeconds"]}, ensure_ascii=False))


if __name__ == "__main__":
    main()
