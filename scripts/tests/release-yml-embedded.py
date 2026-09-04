#!/usr/bin/env python3
"""release.yml 內嵌 shell／python 的行為測試（由 scripts/tests/release-scripts.sh 呼叫）。

YAML 的 block scalar 很容易把 heredoc 縮排吃掉，而 workflow 只有在真的推 tag 時才會跑，
所以「語法看起來對」不算證據。這支測試把每個 `run:` 區塊拿去 `bash -n`，
並把 finalize 的資產盤點與 desktop 的 .sha256 產生器抽出來真的執行：

  - release-provenance-079：資產不齊時 finalize 必須非 0（draft 留著不發布）
  - release-provenance-075：desktop job 必須真的能為 bundle 算出 .sha256，
    且 tauri 沒回報任何 bundle 時不得假綠

輸出以 `FAIL <原因>` 逐行報告，結尾 `DONE`；沒有 PyYAML 時輸出 `SKIP no-pyyaml`。
"""

import json
import os
import subprocess
import sys
import tempfile

try:
    import yaml
except ImportError:  # 環境沒有 PyYAML：誠實回報跳過，不假裝通過
    print("SKIP no-pyyaml")
    sys.exit(0)

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
RELEASE_YML = os.path.join(ROOT, ".github", "workflows", "release.yml")

fails = []
tmp = tempfile.mkdtemp()
rel = yaml.safe_load(open(RELEASE_YML, encoding="utf-8"))


def heredoc(run):
    """取出 `<<'EOF' … EOF` 之間的內容（YAML 縮排已由 block scalar 去掉）。"""
    return run.split("<<'EOF'", 1)[1].split("\n", 1)[1].split("\nEOF", 1)[0]


# ---- 每個 run: 區塊都要能通過 bash -n ---------------------------------------
for job_name, job in rel["jobs"].items():
    for step in job.get("steps", []):
        run = step.get("run")
        if not run:
            continue
        path = os.path.join(tmp, "step.sh")
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(run)
        proc = subprocess.run(
            ["/bin/bash", "-n", path], capture_output=True, text=True, check=False
        )
        if proc.returncode != 0:
            fails.append(
                "%s / %s: %s" % (job_name, step.get("name"), proc.stderr.strip())
            )

# ---- 079：finalize 的資產盤點 ------------------------------------------------
TAG, BARE = "v9.9.9", "9.9.9"
finalize_steps = [s for s in rel["jobs"]["finalize"]["steps"] if s.get("run")]
if not finalize_steps:
    fails.append("release.yml finalize job 沒有任何 run: 步驟")
else:
    inv_py = os.path.join(tmp, "inventory.py")
    with open(inv_py, "w", encoding="utf-8") as fh:
        fh.write(heredoc(finalize_steps[0]["run"]))

    complete = []
    for triple in (
        "x86_64-unknown-linux-gnu",
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "x86_64-pc-windows-msvc",
    ):
        ext = "zip" if "windows" in triple else "tar.gz"
        complete.append("interact-ai-%s-%s.%s" % (TAG, triple, ext))
        complete.append("interact-ai-%s-%s.%s.sha256" % (TAG, triple, ext))
    complete += [
        "orchestrate-adaptive-interaction-skill-%s.zip" % TAG,
        "QUICKSTART.md",
        "install.sh",
        "CHANGELOG.md",
    ]
    bundles = [
        "interaction-control-center_%s_aarch64.dmg" % BARE,
        "interaction-control-center_%s_amd64.AppImage" % BARE,
        "interaction-control-center_%s_amd64.deb" % BARE,
        "interaction-control-center_%s_x64-setup.exe" % BARE,
    ]
    complete += bundles + [b + ".sha256" for b in bundles]

    def inventory(names):
        listing = os.path.join(tmp, "assets.txt")
        with open(listing, "w", encoding="utf-8") as fh:
            fh.write("\n".join(names) + "\n")
        return subprocess.run(
            [sys.executable, inv_py, TAG, listing],
            capture_output=True,
            text=True,
            check=False,
        ).returncode

    if inventory(complete) != 0:
        fails.append("finalize：資產齊全時仍拒絕發布")
    for label, keep in (
        ("Windows CLI 缺席", lambda n: "windows" not in n),
        ("macOS dmg 缺席", lambda n: not n.endswith(".dmg")),
        (
            "AppImage 缺 .sha256",
            lambda n: n != "interaction-control-center_%s_amd64.AppImage.sha256" % BARE,
        ),
        ("skill 包缺席", lambda n: not n.startswith("orchestrate-")),
        ("CLI 少一個 .sha256", lambda n: not n.endswith("aarch64-apple-darwin.tar.gz.sha256")),
    ):
        if inventory([n for n in complete if keep(n)]) == 0:
            fails.append("finalize：%s 時仍把 draft 發布出去" % label)

# ---- 075：desktop job 的 .sha256 產生器 --------------------------------------
checksum_steps = [
    s
    for s in rel["jobs"]["desktop"]["steps"]
    if (s.get("name") or "").startswith("Checksum")
]
if not checksum_steps:
    fails.append("release.yml desktop job 沒有產生 .sha256 的步驟")
else:
    gen_py = os.path.join(tmp, "gen.py")
    with open(gen_py, "w", encoding="utf-8") as fh:
        fh.write(heredoc(checksum_steps[0]["run"]))
    bundle = os.path.join(tmp, "interaction-control-center_9.9.9_aarch64.dmg")
    with open(bundle, "wb") as fh:
        fh.write(b"bundle-bytes")
    proc = subprocess.run(
        [sys.executable, gen_py],
        capture_output=True,
        text=True,
        env=dict(os.environ, ARTIFACT_PATHS=json.dumps([bundle])),
        check=False,
    )
    if proc.returncode != 0 or not os.path.exists(bundle + ".sha256"):
        fails.append("desktop checksum 步驟算不出 .sha256：" + proc.stderr.strip())
    else:
        import hashlib

        expected = hashlib.sha256(b"bundle-bytes").hexdigest()
        got = open(bundle + ".sha256", encoding="utf-8").read().split()[0]
        if got != expected:
            fails.append("desktop checksum 步驟算出的 sha256 不對：%s" % got)
    proc = subprocess.run(
        [sys.executable, gen_py],
        capture_output=True,
        text=True,
        env=dict(os.environ, ARTIFACT_PATHS="[]"),
        check=False,
    )
    if proc.returncode == 0:
        fails.append("desktop checksum 步驟在 tauri 沒回報任何 bundle 時仍成功（假綠）")

for f in fails:
    print("FAIL", f)
print("DONE")
sys.exit(1 if fails else 0)
