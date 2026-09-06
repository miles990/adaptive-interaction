#!/usr/bin/env python3
"""Native work submission/cancellation with repository fixture agents only.

Requires a separately built current App and CLI, an exclusive native UI slot,
and existing Accessibility permission. Example:
  python3 scripts/tests/tauri-work-cancel.py --app /path/to/Interaction.app \
    --cli target/debug/interact-ai --out /tmp/native-work-cancel

Follows e2e/work-delegate.spec.ts: readonly work via the real composer, Codex
turn cancellation, and Claude long-process close. No consent grant/unlock,
real agent invocation, remote work, or claimed human/hardware acceptance.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "crates/interaction-runtime/tests/fixtures"
AX = ROOT / "scripts/lib/tauri-ax.applescript"


def wait(label, check, seconds=30):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except (OSError, ValueError, AssertionError, urllib.error.URLError) as error:
            last = str(error)
        time.sleep(.15)
    raise AssertionError(f"{label}: timed out ({last or 'expected state absent'})")


def stop(proc):
    if proc and proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=12)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def group_members(pgid):
    # Keep only our fixture's group; never save unrelated process information.
    rows = subprocess.check_output(["ps", "-axo", "pid=,pgid=,stat=,comm="], text=True)
    selected = []
    for line in rows.splitlines():
        fields = line.split(None, 3)
        if len(fields) == 4 and int(fields[1]) == pgid and not fields[2].startswith("Z"):
            selected.append({"pid": int(fields[0]), "pgid": int(fields[1]),
                             "state": fields[2], "command": fields[3]})
    return selected


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
    # Keep legacy schema/version and unrelated preferences; only the existing
    # general-mode walkthrough prerequisites are imposed on the isolated copy.
    required = {"companionPack": "plain-text", "companionVisible": True,
                "openControlCenterOnStart": True, "advancedMode": False}
    prefs.update(required)
    return prefs, provenance, required


class WorkJourney:
    def __init__(self, args):
        self.args = args
        self.starting_prefs, self.prefs_fixture, self.prefs_overrides = starting_preferences(args.prefs_fixture)
        self.binary = args.app.resolve() / "Contents/MacOS/interaction-desktop"
        self.cli = args.cli.resolve()
        self.codex = FIXTURES / "fake_codex.sh"
        self.claude = FIXTURES / "fake_claude.sh"
        for executable in (self.binary, self.cli, self.codex, self.claude):
            if not executable.is_file() or not os.access(executable, os.X_OK):
                raise ValueError(f"needs-environment: executable unavailable: {executable}")
        if not shutil.which("osascript"):
            raise ValueError("needs-environment: macOS and existing Accessibility permission required")
        self.artifact_paths = {"binarySha256": self.binary, "cliSha256": self.cli,
                               "codexFixtureSha256": self.codex, "claudeFixtureSha256": self.claude,
                               "driverSha256": Path(__file__).resolve(), "axHelperSha256": AX}
        self.provenance = {key: digest(path) for key, path in self.artifact_paths.items()}
        self.provenance.update(
            sourceSha=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            dirtyTree=bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()),
            provenanceCapturedAt=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()))
        args.out.mkdir(parents=True, exist_ok=False)
        self.home = Path(tempfile.mkdtemp(prefix="aip-native-work-"))
        (self.home / "config").mkdir()
        (self.home / "state").mkdir()
        self.workdirs = {}
        for name, mode in (("codex", "turns"), ("claude", "hang")):
            folder = self.home / (name + "-fixture-work")
            folder.mkdir()
            (folder / "fake-mode").write_text(mode + "\n")
            self.workdirs[name] = folder
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            self.port = sock.getsockname()[1]
        (self.home / "config/interaction.yaml").write_text(
            f"apiHost: 127.0.0.1\napiPort: {self.port}\n")
        # Deliberately override any inherited real-agent setting. Both adapters
        # are pinned even when a task's route changes; no PATH fallback here.
        self.env = {**os.environ, "INTERACT_AI_HOME": str(self.home),
                    "INTERACT_AI_MOBILE_ADVERTISE": "0",
                    "INTERACT_AI_CODEX_BIN": str(self.codex),
                    "INTERACT_AI_CLAUDE_BIN": str(self.claude)}
        self.app = self.daemon = None
        self.token = ""
        self.log = (args.out / "processes.log").open("w")
        self.steps = []
        self.ax_attempts = []
        self.groups = {}
        self.started = time.monotonic()
        self.finished = False

    def api(self, path, body=None):
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.port}" + path,
            data=json.dumps(body).encode() if body is not None else None,
            headers={"Authorization": "Bearer " + self.token, "Content-Type": "application/json"})
        with urllib.request.urlopen(request, timeout=8) as response:
            return json.load(response)

    def ax(self, *args):
        started = time.monotonic()
        result = subprocess.run(["osascript", str(AX), f"pid:{self.app.pid}", *args],
                                capture_output=True, text=True, timeout=55)
        self.ax_attempts.append({"command": list(args), "exit": result.returncode,
                                 "seconds": round(time.monotonic() - started, 3)})
        if result.returncode:
            raise AssertionError(result.stderr.strip())
        return result.stdout.strip()

    def visible(self, label):
        return wait("AX visible: " + label,
                    lambda: self.ax("exists", "AXAny", label) == "yes")

    def click(self, label):
        self.visible(label)
        self.ax("click", "AXButton", label)

    def fill(self, role, label, value):
        # ASCII fixture text/path needs no clipboard or keyboard-layout changes.
        assert value.isascii()
        self.visible(label)
        self.ax("fill", role, label, value)
        wait("AX input readback: " + label,
             lambda: self.ax("value", role, label) == value)

    def capture(self, name):
        text = self.ax("dump")
        if self.token:
            text = text.replace(self.token, "[redacted]")
        (self.args.out / (name + "-ax.txt")).write_text(text)

    def note(self, name, **evidence):
        self.capture(name)
        self.steps.append({"id": name, "status": "completed",
                           "elapsedSeconds": round(time.monotonic() - self.started, 3), **evidence})
        self.save()
        print(name, "completed", flush=True)

    def session(self, label):
        return next((row for row in self.api("/v1/agent-sessions")
                     if row.get("label") == label), None)

    def session_state(self, label, expected):
        def state():
            row = self.session(label)
            return row if row and row.get("state") == expected else None
        return wait("production work state: " + expected, state)

    def submit(self, name, option):
        folder = self.workdirs[name]
        label = "Native fixture " + name + " cancel"
        self.ax("navclick", "3")
        self.fill("AXTextArea", "幫你做什麼", label)
        self.fill("AXTextField", "加入檔案或選擇資料夾", str(folder))
        self.visible("這是哪一種工作")
        self.ax("selectindex", "AXPopUpButton", "這是哪一種工作", str(option))
        self.visible("不會修改：這次只看不改")
        self.click("開始")
        # The hang fixture accepts stdin but emits no task-started/progress.
        # Its honest session state remains created; Codex emits a real turn event.
        record = self.session_state(label, "active" if name == "codex" else "created")
        expected_agent = "codex" if name == "codex" else "claude-code"
        assert record["agentId"] == expected_agent, record
        assert record["allowWrite"] is False, record
        assert record["dataScope"] == ["workspace:" + str(folder)], record
        assert record["toolScope"] == [] and record["consentScope"] == [], record
        wait("fixture PID file", lambda: (folder / "fake-pid").is_file())
        pid = int((folder / "fake-pid").read_text().strip())
        pgid = os.getpgid(pid)
        assert pgid == pid and pgid != os.getpgrp(), "fixture must own a separate process group"
        self.groups[name] = pgid
        members = group_members(pgid)
        assert members
        # Capture transport input, not just an active session with no task.
        if name == "claude":
            wait("task reached the hanging fixture",
                 lambda: (folder / "fake-input").is_file()
                 and label in (folder / "fake-input").read_text())
        else:
            self.visible("處理中")
            wait("Codex thread handshake", lambda: (folder / "fake-thread-start").is_file())
        self.note(name + "-readonly-work-submitted", session=record, fixtureProcesses=members)
        return label, folder, pgid

    def run(self):
        failure = None
        try:
            self.daemon = subprocess.Popen(
                [str(self.cli), "serve", "--host", "127.0.0.1", "--port", str(self.port)],
                env=self.env, stdout=self.log, stderr=self.log)
            wait("isolated API token", lambda: (self.home / "state/api-token").is_file())
            self.token = (self.home / "state/api-token").read_text().strip()
            wait("production daemon ready", lambda: self.api("/ready"))
            discoveries = wait("fixture discoveries", lambda: self.api("/v1/agents"))
            for agent in discoveries["agents"]:
                assert Path(agent["binaryPath"]).resolve() in (self.codex, self.claude), agent
                assert agent["found"] is True and agent["loggedIn"] is True, agent
            assert len(discoveries["agents"]) == 2, discoveries
            # Same setup as the existing native preset runner: no grants.
            self.api("/v1/onboarding/commit", {})
            (self.home / "state/desktop.json").write_text(json.dumps(self.starting_prefs))
            self.app = subprocess.Popen([str(self.binary)], env=self.env, stdout=self.log, stderr=self.log)
            wait("native window", lambda: "Interaction Control Center" in self.ax("windows"))
            wait("native primary navigation", lambda: self.ax("navclick", "3"))

            label, folder, pgid = self.submit("codex", 2)
            self.click("暫停／中斷目前工作")
            cancelled = self.session_state(label, "cancelled")
            self.visible("已取消")  # no navigation/reload: SSE must update it
            wait("actual turn/interrupt at fixture", lambda: (folder / "fake-turn-interrupt").is_file())
            interruption = json.loads((folder / "fake-turn-interrupt").read_text())
            assert interruption["method"] == "turn/interrupt"
            self.note("codex-turn-cancelled", session=cancelled, fixtureInterrupt=interruption,
                      remainingProcesses=group_members(pgid),
                      evidence="Cancelled turn; app-server may remain alive for the existing session")

            label, folder, pgid = self.submit("claude", 1)
            def sleeping():
                members = group_members(pgid)
                return members if any("sleep" in row["command"] for row in members) else None
            before = wait("fixture entered its long-running sleep", sleeping)
            self.click("關閉")
            closed = self.session_state(label, "closed")
            self.visible("工作階段已關閉（已要求終止子程序）")
            wait("fixture process group exited after native close", lambda: not group_members(pgid), 15)
            self.note("claude-long-work-closed-and-processes-exited", session=closed,
                      processesBeforeClose=before, processesAfterClose=group_members(pgid))
            self.finished = True
        except Exception as error:
            failure = str(error)
            if self.app and self.app.poll() is None:
                try:
                    self.capture("failed")
                except Exception:
                    pass
            print("failed:", failure, flush=True)
        finally:
            stop(self.app)
            stop(self.daemon)
            # Runtime shutdown owns cleanup. A forced test cleanup is explicitly
            # a failure and never turns the UI process-stop check into a pass.
            for name, pgid in self.groups.items():
                remaining = group_members(pgid)
                if remaining:
                    failure = failure or f"owned {name} fixture group survived daemon shutdown"
                    try:
                        os.killpg(pgid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
            self.log.close()
            log_path = self.args.out / "processes.log"
            log_path.write_text(log_path.read_text(errors="replace").replace(self.token, "[redacted]")
                                if self.token else log_path.read_text(errors="replace"))
            changed = self.changed_artifacts()
            failure = failure or ("pinned artifacts changed during run: " + ", ".join(changed) if changed else None)
            self.save(failure)
            shutil.rmtree(self.home)
        return 1 if failure else 0

    def changed_artifacts(self):
        return [key for key, path in self.artifact_paths.items()
                if not path.is_file() or digest(path) != self.provenance[key]]

    def save(self, error=None):
        changed = self.changed_artifacts()
        error = error or ("pinned artifacts changed during run: " + ", ".join(changed) if changed else None)
        document = {
            "evidenceLevel": "native Tauri AX + production daemon + repository fixture agents",
            "realAgentExecuted": False, "humanEvaluation": False, "consentGranted": False,
            "safetyUnlocked": False, **self.provenance,
            "artifactVerification": {"unchanged": not changed, "changed": changed},
            "fixtureSha256": {"codex": self.provenance["codexFixtureSha256"],
                              "claude": self.provenance["claudeFixtureSha256"]},
            "prefsFixture": self.prefs_fixture, "startingPreferences": self.starting_prefs,
            "casePreferenceOverrides": self.prefs_overrides,
            "steps": self.steps, "ax": self.ax_attempts,
            "seconds": round(time.monotonic() - self.started, 3),
            "result": "failed" if error else "completed" if self.finished else "running",
            "error": error, "limitations": ["No real AI or remote task success",
                "Codex turn cancellation does not claim the app-server process exited",
                "Process termination is checked on the separate Claude hang/close case"],
        }
        text = json.dumps(document, ensure_ascii=False, indent=2)
        if self.token:
            text = text.replace(self.token, "[redacted]")
        (self.args.out / "result.json").write_text(text + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--cli", type=Path, default=ROOT / "target/debug/interact-ai")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--prefs-fixture", type=Path,
                        help="JSON preferences fixture copied only to the isolated home; case prerequisites override it")
    try:
        return WorkJourney(parser.parse_args()).run()
    except (ValueError, OSError) as error:
        print(str(error), flush=True)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
