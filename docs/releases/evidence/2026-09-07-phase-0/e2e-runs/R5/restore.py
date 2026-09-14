#!/usr/bin/env python3
"""Mimic BackupSection.tsx restoreBackup(): POST /v1/memory per item with
exactly the fields the UI sends (layer/kind/title/content/provenance/
confidence/tags/agentVisibility/agentDenylist/retention). No asAgent field is
sent by the UI's restore path (it is not read from source), so restored items
are always human-authored regardless of original createdBy.
"""
import json
import sys
import urllib.request

EXPORT_FILE = sys.argv[1]
API = sys.argv[2]
TOKEN = sys.argv[3]
OUT_LOG = sys.argv[4]

with open(EXPORT_FILE) as f:
    export = json.load(f)

items = export["items"]
restored = []
errors = []

log_lines = []

for idx, source in enumerate(items):
    body = {
        "layer": source.get("layer"),
        "kind": source.get("kind"),
        "title": source.get("title"),
        "content": source.get("content"),
        "provenance": source.get("provenance"),
        "confidence": source.get("confidence"),
        "tags": source.get("tags"),
        "agentVisibility": source.get("agentVisibility"),
        "agentDenylist": source.get("agentDenylist"),
        "retention": source.get("retention"),
    }
    data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(
        f"{API}/v1/memory",
        data=data,
        method="POST",
        headers={"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req) as resp:
            result = json.loads(resp.read())
            restored.append({"sourceMemoryId": source.get("memoryId"), "newMemoryId": result.get("memoryId"), "result": result})
            log_lines.append(f"item {idx} ({source.get('memoryId')}) -> OK new id {result.get('memoryId')}")
    except urllib.error.HTTPError as e:
        err_body = e.read().decode("utf-8")
        errors.append({"index": idx, "status": e.code, "body": err_body})
        log_lines.append(f"item {idx} ({source.get('memoryId')}) -> ERROR {e.code} {err_body}")
        # BackupSection stops the loop entirely on first error (throw inside for-loop)
        break

with open(OUT_LOG, "w") as f:
    f.write("\n".join(log_lines) + "\n")
    f.write(f"\nrestored_count={len(restored)}\n")
    f.write(f"errors={json.dumps(errors)}\n")

print(json.dumps({"restored": restored, "errors": errors}, indent=2))
