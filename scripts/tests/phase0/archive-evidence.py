#!/usr/bin/env python3
"""把 scratchpad 裡的階段 0 原始證據歸檔進 repo（docs/releases/evidence/<date>-phase-0/…）。

    python3 scripts/tests/phase0/archive-evidence.py --checkpoint <sha> --dest docs/releases/evidence/2026-09-07-phase-0 \
        --note "…" SRC_DIR:DEST_SUBDIR [SRC_DIR:DEST_SUBDIR ...] [--exclude-dir home --exclude-dir workdir]

規則（沿用 2026-09-06-convergence 的 manifest 慣例）：
  - .log 存成 .txt、.stdout／.stderr 存成 .stdout.txt／.stderr.txt（不互相覆蓋）；文字檔正規化行尾空白與多餘末行；JSON 原樣保存。
  - 一律略過名稱含 `home` 的目錄（含 api-token／api-agent-token／SQLite）與其他 --exclude-dir；FIFO／socket／symlink 不歸檔；超過 --max-bytes 的檔只記 hash 不複製。
  - 憑證掃描：每個來源 run 目錄若有 home/state/api-token、api-agent-token，其值不得出現在任何被歸檔的檔案裡；
    出現即中止（不做自動改寫，讓人看清楚是哪個檔）。另外掃 `Bearer <token>` 與 `sk-`／`ghp_` 形狀的字串。
  - artifact-manifest.json 記 original／path／originalSha256／archivedSha256／bytes／normalization。
"""
import argparse, hashlib, json, os, re, stat, sys

TEXT_EXT = {".log", ".stdout", ".stderr", ".txt", ".md", ".tsv", ".jsonl", ".yaml", ".yml", ".py", ".sh", ".applescript"}
RENAME = {".log": ".txt", ".stdout": ".stdout.txt", ".stderr": ".stderr.txt"}

def sha256(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()

def normalize_text(b: bytes) -> bytes:
    text = b.decode("utf-8", "replace")
    lines = [ln.rstrip() for ln in text.split("\n")]
    while lines and lines[-1] == "":
        lines.pop()
    return ("\n".join(lines) + "\n").encode("utf-8")

def collect_secrets(src_root: str):
    secrets = set()
    for dirpath, dirnames, filenames in os.walk(src_root):
        for fn in filenames:
            if fn in ("api-token", "api-agent-token"):
                p = os.path.join(dirpath, fn)
                if os.path.isfile(p) and not os.path.islink(p):
                    v = open(p, "rb").read().strip()
                    if len(v) >= 16:
                        secrets.add(v)
    return secrets

SECRET_SHAPES = [re.compile(rb"Bearer [A-Za-z0-9_\-\.=]{16,}"), re.compile(rb"\bsk-[A-Za-z0-9]{16,}"), re.compile(rb"\bghp_[A-Za-z0-9]{20,}")]

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--checkpoint", required=True)
    ap.add_argument("--dest", required=True)
    ap.add_argument("--note", default="")
    ap.add_argument("--exclude-dir", action="append", default=["home", "__pycache__", "node_modules", "ios-sim-build"])
    ap.add_argument("--max-bytes", type=int, default=2_000_000)
    ap.add_argument("pairs", nargs="+", help="SRC_DIR:DEST_SUBDIR")
    a = ap.parse_args()
    os.makedirs(a.dest, exist_ok=True)
    manifest_path = os.path.join(a.dest, "artifact-manifest.json")
    files = []
    problems = []
    written = []  # 這一次執行真的寫進 dest 的檔；中止時只清這些，不動 dest 原有內容
    for pair in a.pairs:
        src, sub = pair.split(":", 1)
        src = os.path.abspath(src)
        secrets = collect_secrets(src)
        for dirpath, dirnames, filenames in os.walk(src):
            dirnames[:] = sorted(d for d in dirnames if d not in a.exclude_dir and "home" not in d)
            for fn in sorted(filenames):
                if fn in ("api-token", "api-agent-token") or fn.endswith(".db") or fn.endswith(".db-wal") or fn.endswith(".db-shm"):
                    continue  # 憑證與 SQLite 永遠不進 repo
                sp = os.path.join(dirpath, fn)
                st_ = os.lstat(sp)
                if not stat.S_ISREG(st_.st_mode):
                    continue  # FIFO／socket／symlink：讀了會卡住或指到外面，一律不歸檔
                rel = os.path.relpath(sp, src)
                raw = open(sp, "rb").read()
                ext = os.path.splitext(fn)[1]
                for sec in secrets:
                    if sec and sec in raw:
                        problems.append(f"secret from home/state found in {sp}")
                for shape in SECRET_SHAPES:
                    if shape.search(raw) and ext not in (".py", ".sh"):
                        problems.append(f"credential-shaped text in {sp}: {shape.pattern!r}")
                out_rel = os.path.join(sub, os.path.splitext(rel)[0] + RENAME[ext]) if ext in RENAME else os.path.join(sub, rel)
                entry = {"original": sp, "path": out_rel, "originalSha256": sha256(raw), "bytes": len(raw)}
                if len(raw) > a.max_bytes:
                    entry["archivedSha256"] = None
                    entry["normalization"] = f"not copied (>{a.max_bytes} bytes); hash only"
                    files.append(entry); continue
                data = normalize_text(raw) if (ext in TEXT_EXT and ext != ".jsonl") else raw
                entry["archivedSha256"] = sha256(data)
                entry["normalization"] = "text trailing whitespace / EOF blank lines normalized" if data is not raw else "byte-identical"
                dp = os.path.join(a.dest, out_rel)
                os.makedirs(os.path.dirname(dp), exist_ok=True)
                open(dp, "wb").write(data)
                written.append(dp)
                files.append(entry)
    if problems:
        for p in problems:
            print("REFUSED:", p, file=sys.stderr)
        # 不留半成品，但只收回這一次寫入的檔（dest 可能是 repo 內已有其他證據的目錄，
        # 整個 rmtree 會把不屬於這次執行的東西一起刪掉）。空掉的子目錄順手移除。
        for dp in written:
            try:
                os.remove(dp)
            except FileNotFoundError:
                pass
        for dp in sorted({os.path.dirname(p) for p in written}, key=len, reverse=True):
            while dp.startswith(os.path.abspath(a.dest)) and dp != os.path.abspath(a.dest):
                try:
                    os.rmdir(dp)
                except OSError:
                    break
                dp = os.path.dirname(dp)
        print(f"removed {len(written)} files written by this run; pre-existing content in {a.dest} untouched", file=sys.stderr)
        sys.exit(2)
    manifest = {"evidenceSourceCheckpoint": a.checkpoint, "note": a.note, "files": files}
    json.dump(manifest, open(manifest_path, "w"), ensure_ascii=False, indent=1)
    print(f"archived {len(files)} files → {a.dest} (manifest {manifest_path})")

if __name__ == "__main__":
    main()
