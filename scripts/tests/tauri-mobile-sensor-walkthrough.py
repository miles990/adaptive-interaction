#!/usr/bin/env python3
"""Native Tauri + production daemon journeys using a simulated iPhone.

Build inputs separately, then run in an exclusive native UI slot:
  cargo build -p interaction-cli
  cargo build -p interaction-runtime --example fake_iphone
  python3 scripts/tests/tauri-mobile-sensor-walkthrough.py \
    --app /path/to/Interaction.app --out /tmp/native-mobile-sensor

This drives the existing AX helper, never a browser mock. The iPhone is the
committed fake_iphone process (pinned TLS/HMAC/wire), not hardware. It never
grants consent, enables a receptor, unlocks safety, or captures real audio.
The sensor scenario reports a fictional micLevel flag and deliberately does
not acknowledge stop-all. All runtime data is isolated and removed on exit.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
AX = ROOT / "scripts/lib/tauri-ax.applescript"
MIC = "iphone.mic-level"
PHONE_LABEL = "模擬 iPhone（fixture）"


def wait(label, check, seconds=30):
    end = time.monotonic() + seconds
    last = None
    while time.monotonic() < end:
        try:
            value = check()
            if value:
                return value
        except (OSError, ValueError, AssertionError, urllib.error.URLError) as exc:
            last = str(exc)
        time.sleep(.15)
    raise AssertionError(f"{label}: timed out ({last or 'expected state absent'})")


def stop(proc):
    if proc and proc.poll() is None:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=12)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class Phone:
    """Read the fixture's JSONL in memory; never persist its device token."""

    def __init__(self, executable, pairing, log, name=PHONE_LABEL):
        self.events = []
        self.device_id = None
        self.error = None
        self.lock = threading.Lock()
        self.proc = subprocess.Popen(
            [str(executable), "--port", str(pairing["port"]),
             "--fingerprint", pairing["fingerprint"], "--code", pairing["code"],
             "--name", name], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=log, text=True, bufsize=1,
        )
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()
        try:
            wait("fixture pairing", self.paired)
        except Exception:
            stop(self.proc)
            self.reader.join(timeout=2)
            raise

    def read(self):
        for line in self.proc.stdout:
            try:
                value = json.loads(line)
            except ValueError:
                continue
            with self.lock:
                if "deviceId" in value and "deviceToken" in value:
                    self.device_id = value["deviceId"]
                    # Only the child retains this secret for its reconnect op.
                    self.events.append({"event": "paired", "deviceId": self.device_id})
                else:
                    self.events.append(value)

    def paired(self):
        if self.proc.poll() is not None:
            raise RuntimeError("fake_iphone exited before pairing; inspect fixture log")
        return self.device_id

    def send(self, op, **fields):
        if self.proc.poll() is not None:
            raise RuntimeError("fixture exited before " + op)
        self.proc.stdin.write(json.dumps({"op": op, **fields}) + "\n")
        self.proc.stdin.flush()

    def mark(self):
        with self.lock:
            return len(self.events)

    def event(self, predicate, start=0):
        def find():
            with self.lock:
                return next((row for row in self.events[start:] if predicate(row)), None)
        return wait("fixture event", find)

    def aip(self, predicate, start=0):
        return self.event(lambda row: row.get("event") == "aip"
                          and predicate(row.get("envelope", {})), start)["envelope"]

    def negotiate(self):
        start = self.mark()
        self.send("aip-capability")
        self.aip(lambda frame: frame.get("messageType") == "capability", start)
        return self.aip(lambda frame: frame.get("messageType") == "state"
                        and frame.get("payload", {}).get("kind") == "snapshot", start)["payload"]


def starting_preferences(fixture):
    prefs = {"schemaVersion": 3}
    provenance = None
    if fixture is not None:
        fixture = fixture.resolve()
        raw = fixture.read_bytes()
        loaded = json.loads(raw)
        if not isinstance(loaded, dict):
            raise ValueError("--prefs-fixture must contain a JSON preferences object")
        prefs.update(loaded)
        provenance = {"path": str(fixture), "sha256": hashlib.sha256(raw).hexdigest()}
    # Keep the fixture's schema version and unrelated preferences for migration;
    # these fields are the existing general-mode walkthrough prerequisites.
    required = {"companionPack": "plain-text", "companionVisible": True,
                "companionExpressiveness": "natural", "openControlCenterOnStart": True,
                "advancedMode": False}
    prefs.update(required)
    return prefs, provenance, required


class Journey:
    def __init__(self, args):
        self.args = args
        self.starting_prefs, self.prefs_fixture, self.prefs_overrides = starting_preferences(args.prefs_fixture)
        self.binary = args.app.resolve() / "Contents/MacOS/interaction-desktop"
        self.cli = args.cli.resolve()
        self.fake = args.fake_iphone.resolve()
        for executable in (self.binary, self.cli, self.fake):
            if not executable.is_file() or not os.access(executable, os.X_OK):
                raise ValueError(f"needs-environment: executable missing: {executable}")
        if not shutil.which("osascript"):
            raise ValueError("needs-environment: macOS osascript and existing Accessibility permission required")
        self.artifact_paths = {"binarySha256": self.binary, "cliSha256": self.cli,
                               "fakeIphoneSha256": self.fake, "driverSha256": Path(__file__).resolve(),
                               "axHelperSha256": AX}
        self.provenance = {key: sha(path) for key, path in self.artifact_paths.items()}
        self.provenance.update(
            sourceSha=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            dirtyTree=bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()),
            provenanceCapturedAt=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()))
        args.out.mkdir(parents=True, exist_ok=False)
        self.home = Path(tempfile.mkdtemp(prefix="aip-native-mobile-sensor-"))
        (self.home / "config").mkdir()
        (self.home / "state").mkdir()
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            self.port = sock.getsockname()[1]
        (self.home / "config/interaction.yaml").write_text(
            f"apiHost: 127.0.0.1\napiPort: {self.port}\n")
        self.env = {**os.environ, "INTERACT_AI_HOME": str(self.home),
                    "INTERACT_AI_MOBILE_ADVERTISE": "0"}
        self.token = ""
        self.codes = []
        self.app = self.daemon = None
        self.phones = []
        self.log = (args.out / "processes.log").open("w")
        self.steps = []
        self.ax_attempts = []
        self.started = time.monotonic()
        self.finished = False

    def api(self, path):
        # The only mutation below is the same isolated onboarding setup used by
        # tauri-preset-recovery.py; all task actions use the production native UI.
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.port}" + path,
            headers={"Authorization": "Bearer " + self.token})
        with urllib.request.urlopen(request, timeout=8) as response:
            return json.load(response)

    def start_daemon(self):
        self.daemon = subprocess.Popen(
            [str(self.cli), "serve", "--host", "127.0.0.1", "--port", str(self.port)],
            env=self.env, stdout=self.log, stderr=self.log)
        wait("daemon token", lambda: (self.home / "state/api-token").is_file())
        self.token = (self.home / "state/api-token").read_text().strip()
        wait("production daemon ready", lambda: self.api("/ready"))
        wait("production daemon status", lambda: self.api("/v1/status"))

    def start_app(self):
        self.app = subprocess.Popen([str(self.binary)], env=self.env,
                                    stdout=self.log, stderr=self.log)
        wait("native Tauri window", lambda: "Interaction Control Center" in self.ax("windows"))
        wait("native primary navigation", lambda: self.ax("navclick", "1"))

    def ax(self, *args):
        started = time.monotonic()
        result = subprocess.run(
            ["osascript", str(AX), f"pid:{self.app.pid}", *args],
            capture_output=True, text=True, timeout=55)
        self.ax_attempts.append({"command": list(args), "exit": result.returncode,
                                 "seconds": round(time.monotonic() - started, 3)})
        if result.returncode:
            raise AssertionError("AX failed: " + result.stderr.strip())
        return result.stdout.strip()

    def visible(self, text):
        return wait("native visible: " + text,
                    lambda: self.ax("exists", "AXAny", text) == "yes")

    def click(self, text, role="AXButton"):
        # Readiness is retried; mutations run once. A delayed response must not
        # turn retries into duplicate pairing or confirmation actions.
        self.visible(text)
        return self.ax("click", role, text)

    def connect_page(self, pairing=False):
        self.ax("navclick", "4")
        self.click("裝置與能力", "AXAny")
        if pairing:
            self.click("裝置與來源", "AXAny")

    def redact(self, text):
        for secret in [self.token, *self.codes]:
            if secret:
                text = text.replace(secret, "[redacted]")
        return re.sub(r'("(?:deviceToken|token|code)"\s*:\s*")[^"]*',
                      r'\1[redacted]', text)

    def capture(self, name):
        # AX dump is scoped to our owned process, not the user's desktop. No
        # screenshot is taken while one-time pairing material may be visible.
        text = self.redact(self.ax("dump"))
        (self.args.out / (name + "-ax.txt")).write_text(text)
        return text

    def note(self, name, **evidence):
        self.capture(name)
        self.steps.append({"id": name, "status": "completed",
                           "elapsedSeconds": round(time.monotonic() - self.started, 3),
                           **evidence})
        self.save()
        print(name, "completed", flush=True)

    def status(self):
        return self.api("/v1/status")

    def session(self):
        return self.api("/v1/character-session")["payload"]

    def device(self, phone):
        return next((device for device in self.api("/v1/mobile/status")["devices"]
                     if device["deviceId"] == phone.device_id), None)

    def pair(self):
        self.connect_page(pairing=True)
        self.click("開始配對（5 分鐘內有效）")
        def pairing_code():
            text = self.ax("dump")
            match = re.search(r"輸入配對碼[：:]\s*(?:AX\w+:\s*)?(\d{6})", text)
            return match.group(1) if match else None
        code = wait("one-time pairing code visible in native AX", pairing_code)
        self.codes.append(code)
        transport = self.api("/v1/mobile/status")
        phone = Phone(self.fake, {"code": code, "port": transport["port"],
                                 "fingerprint": transport["fingerprint"]}, self.log)
        self.phones.append(phone)
        wait("paired production device", lambda: (self.device(phone) or {}).get("connected"))
        self.visible(PHONE_LABEL)
        return phone

    def mobile(self):
        phone = self.pair()
        snapshot = phone.negotiate()
        self.ax("navclick", "2")
        self.visible(PHONE_LABEL)
        self.note("mobile-pair-and-negotiate", deviceId=phone.device_id,
                  snapshotRevision=snapshot["revision"],
                  receiptEvidence="legacy fixture: does not send state-applied receipts")

        before = self.session()["revision"]
        start = phone.mark()
        phone.send("aip-touch", kind="tap", messageId="native-fixture-touch")
        result = phone.aip(lambda frame: frame.get("messageType") == "result", start)
        assert result["payload"]["status"] == "applied", result
        patch = phone.aip(lambda frame: frame.get("messageType") == "state"
                          and frame.get("payload", {}).get("kind") == "patch", start)
        current = wait("authoritative revision after fixture touch",
                       lambda: (value if (value := self.session())["revision"] > before else None))
        assert current["state"]["lastInteraction"]["name"] == "character.interaction.touch"
        self.visible("摸了摸角色")
        self.note("mobile-bidirectional-semantic", result=result["payload"],
                  revisionBefore=before, revisionAfter=current["revision"],
                  hostPatchRevision=patch["payload"]["revision"],
                  evidence="fixture input -> authoritative state -> native AX; host patch -> fixture")

        start = phone.mark()
        phone.send("disconnect")
        phone.event(lambda row: row.get("event") == "disconnected", start)
        wait("production mobile disconnected", lambda: (self.device(phone) or {}).get("connected") is False)
        self.connect_page()
        self.visible("未連線（能力不可用）")
        self.note("mobile-disconnect", deviceId=phone.device_id)
        start = phone.mark()
        phone.send("reconnect")
        phone.event(lambda row: row.get("event") == "connected", start)
        phone.negotiate()
        wait("production mobile reconnected", lambda: (self.device(phone) or {}).get("connected"))
        self.visible("已連線")
        self.note("mobile-reconnect", deviceId=phone.device_id)

        start = phone.mark()
        self.click("移除此手機")
        self.click("確定移除？（立即斷線，要再用必須重新配對）")
        wait("revocation persisted in production device list", lambda: self.device(phone) is None)
        phone.event(lambda row: row.get("event") == "disconnected", start)
        rejected_before = self.api("/v1/mobile/status")["heartbeat"]["failedAuths"]
        start = phone.mark()
        phone.send("reconnect")
        rejection = phone.event(lambda row: row.get("event") == "auth-fail", start)
        wait("old device token rejected by production host",
             lambda: self.api("/v1/mobile/status")["heartbeat"]["failedAuths"] > rejected_before)
        assert self.device(phone) is None
        self.note("mobile-remove-old-token-rejected", reason=rejection.get("reason"),
                  nativeAction="two-step remove confirmation", oldDeviceId=phone.device_id)
        replacement = self.pair()
        assert replacement.device_id != phone.device_id
        replacement.negotiate()
        self.note("mobile-pair-again", deviceId=replacement.device_id)
        return replacement

    def sensor(self, phone):
        self.connect_page()
        phone.send("status", micLevel=True, permissions={"microphone": "notDetermined"})
        wait("fixture micLevel self-report", lambda: (self.device(phone) or {}).get("sensors", {}).get("micLevel") is True)
        start = phone.mark()
        self.click("停止感測")
        phone.event(lambda row: row.get("event") == "stop-all" and row.get("sensors") is True, start)
        def unknown_capture():
            return next((sensor for sensor in self.status().get("activeSensors", [])
                         if sensor.get("kind") == MIC and sensor.get("state") == "stop-unknown"), None)
        wait("unacknowledged device stop is unknown", unknown_capture)
        self.visible("結果不確定")
        self.note("sensor-stop-unknown", capture=unknown_capture(),
                  setup="simulated self-report; no receptor enable or consent grant")
        # Direct PhoneDeviceCard removal must preserve this unknown by itself.
        # An extra global stop here would mask a missing journal binding.
        self.click("移除此手機")
        self.click("確定移除？（立即斷線，要再用必須重新配對）")
        wait("streaming fixture removed", lambda: self.device(phone) is None)
        before_restart = wait("direct removal retains unconfirmed mic stop",
                              lambda: [entry for entry in self.status().get("unresolvedStops", [])
                                       if MIC in entry.get("sensors", [])])
        self.visible("筆感測停止沒有人確認")
        self.note("sensor-direct-remove-retains-unknown", unresolvedStops=before_restart,
                  priorGlobalStop=False)

        old_pid = self.daemon.pid
        stop(self.app)
        self.app = None
        stop(self.daemon)
        assert self.daemon.poll() is not None
        self.daemon = None
        self.start_daemon()
        assert self.daemon.pid != old_pid
        def unresolved():
            return [entry for entry in self.status().get("unresolvedStops", [])
                    if MIC in entry.get("sensors", [])]
        restored = wait("unknown survives entire daemon process restart", unresolved)
        assert self.status().get("activeSensors", []) == []
        self.start_app()
        self.visible("筆感測停止沒有人確認")
        self.note("sensor-unknown-after-daemon-restart", previousDaemonPid=old_pid,
                  currentDaemonPid=self.daemon.pid, unresolvedStops=restored,
                  unresolvedStopHealth=self.status().get("unresolvedStopHealth"),
                  humanDismissal=False, deviceStopAcknowledgement=False)

    def global_sensor(self):
        # Separate fresh connection: this checks the global action without
        # using it to prepare the per-device removal/restart scenario above.
        phone = self.pair()
        phone.send("status", micLevel=True, permissions={"microphone": "notDetermined"})
        wait("global scenario fixture self-report",
             lambda: (self.device(phone) or {}).get("sensors", {}).get("micLevel") is True)
        start = phone.mark()
        self.ax("navclick", "1")
        self.click("停止所有感測")
        phone.event(lambda row: row.get("event") == "stop-all" and row.get("sensors") is True, start)
        def unknown_capture():
            return next((sensor for sensor in self.status().get("activeSensors", [])
                         if sensor.get("startedBy") == "iphone:" + phone.device_id
                         and sensor.get("state") == "stop-unknown"), None)
        capture = wait("global stop unacknowledged by fixture", unknown_capture)
        self.visible("已要求停止")
        assert self.ax("exists", "AXAny", "已停止感測。") == "no"
        self.note("sensor-global-stop-separate-fixture", capture=capture)

    def changed_artifacts(self):
        return [key for key, path in self.artifact_paths.items()
                if not path.is_file() or sha(path) != self.provenance[key]]

    def save(self, error=None):
        changed = self.changed_artifacts()
        error = error or ("pinned artifacts changed during run: " + ", ".join(changed) if changed else None)
        document = {
            "evidenceLevel": "native Tauri AX + production daemon + fake_iphone simulator",
            "humanOrHardwareEvidence": False, "realMicrophoneCapture": False,
            "consentGranted": False, "safetyUnlocked": False,
            **self.provenance, "artifactVerification": {"unchanged": not changed, "changed": changed},
            "steps": self.steps,
            "prefsFixture": self.prefs_fixture, "startingPreferences": self.starting_prefs,
            "casePreferenceOverrides": self.prefs_overrides,
            "ax": self.ax_attempts, "seconds": round(time.monotonic() - self.started, 3),
            "result": "failed" if error else "running", "error": error,
            "limitations": ["No physical iPhone, real sensor, or human evaluation",
                            "Legacy fake_iphone has no state-applied receipt capability",
                            "No actual long-running agent work or backup/restore in this runner"],
        }
        if not error and self.finished:
            document["result"] = "completed"
        (self.args.out / "result.json").write_text(self.redact(json.dumps(document, ensure_ascii=False, indent=2)) + "\n")
        for index, phone in enumerate(self.phones):
            with phone.lock:
                events = list(phone.events)
            (self.args.out / f"phone-{index + 1}-events.json").write_text(
                self.redact(json.dumps(events, ensure_ascii=False, indent=2)) + "\n")

    def run(self):
        failure = None
        try:
            self.start_daemon()
            request = urllib.request.Request(
                f"http://127.0.0.1:{self.port}/v1/onboarding/commit", data=b"{}",
                headers={"Authorization": "Bearer " + self.token, "Content-Type": "application/json"}, method="POST")
            with urllib.request.urlopen(request, timeout=10) as response:
                assert response.status == 200
            (self.home / "state/desktop.json").write_text(json.dumps(self.starting_prefs))
            self.start_app()
            phone = self.mobile()
            self.sensor(phone)
            self.global_sensor()
            self.finished = True
        except Exception as exc:
            failure = str(exc)
            if self.app and self.app.poll() is None:
                try:
                    self.capture("failed")
                except Exception:
                    pass
            print("failed:", self.redact(failure), flush=True)
        finally:
            for phone in self.phones:
                stop(phone.proc)
                phone.reader.join(timeout=2)
            stop(self.app)
            stop(self.daemon)
            self.log.close()
            log_path = self.args.out / "processes.log"
            log_path.write_text(self.redact(log_path.read_text(errors="replace")))
            changed = self.changed_artifacts()
            failure = failure or ("pinned artifacts changed during run: " + ", ".join(changed) if changed else None)
            self.save(failure)
            shutil.rmtree(self.home)
        return 1 if failure else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--cli", type=Path, default=ROOT / "target/debug/interact-ai")
    parser.add_argument("--fake-iphone", type=Path, default=ROOT / "target/debug/examples/fake_iphone")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--prefs-fixture", type=Path,
                        help="JSON preferences fixture copied only to the isolated home; case prerequisites override it")
    args = parser.parse_args()
    try:
        return Journey(args).run()
    except (ValueError, OSError) as exc:
        print(str(exc), flush=True)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
