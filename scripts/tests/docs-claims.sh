#!/usr/bin/env bash
# 文件誠實度 lint：文件裡對「程式碼現況」的可驗證陳述，必須與 repo 現況一致。
#
#   bash scripts/tests/docs-claims.sh
#
# 覆蓋：
#   - CHANGELOG 最上層非空段（Unreleased 或剛命名的版本段）不得宣稱已落地的功能「尚未落地」（evidence-honesty-012）
#   - ARCHITECTURE.md 不得以「搜尋不到對應實作」宣告已存在的模組不存在（evidence-honesty-013）
#   - threat-model.md 不得用會漂移的硬編行號引用 session.rs::gate()（evidence-honesty-014）
#   - AIP §10 的 EvidenceClass 措辭必須與「有沒有生產／消費端」一致（evidence-honesty-016）
#   - 安裝文件對完整性驗證／簽章／平台覆蓋的宣稱必須與實作一致（release-provenance-074/075/080）
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
echo "docs-claims lint @ $ROOT"

python3 - <<'PY'
import os, re, sys

fails = []
checks = 0

def read(p):
    return open(p, encoding="utf-8").read()

def need(cond, msg):
    global checks
    checks += 1
    if not cond:
        fails.append(msg)

# ---- evidence-honesty-012：CHANGELOG [Unreleased] --------------------------
ch = read("CHANGELOG.md")
# 最上層「非空」段：release-prepare 會把 Unreleased 改名成版本段並留下一個空的 Unreleased 標題，
# 此時要檢查的是那個剛命名的版本段（與 src-tauri 的 changelog_topmost_section 同一規則）。
def topmost_section(text):
    parts = text.split("\n## [")
    for part in parts[1:]:
        body = part.split("\n", 1)[1] if "\n" in part else ""
        if body.strip():
            return body
    return ""
unreleased = topmost_section(ch)
def version_section(text, version):
    marker = "\n## [%s]" % version
    if marker not in text:
        return ""
    body = text.split(marker, 1)[1]
    body = body.split("\n", 1)[1] if "\n" in body else ""
    return body.split("\n## [", 1)[0]
# 這三個功能在 0.6.0 落地：條目必須在 `## [0.6.0]` 段（綁版本），不是在「目前最上層的段落」——
# 後者會逼每一個新的 [Unreleased] 段反覆抄一次（ea7de59 因此紅過一次 CI）。行為本身由
# executable tests 保護（character_session_loop.rs／character-sync-card.test.tsx／SessionClientTests.swift）。
landed = {
    "Runtime Session Host": ("0.6.0", "crates/interaction-runtime/src/character_session.rs"),
    "桌面同步卡": ("0.6.0", "apps/interaction-desktop/src/components/CharacterSyncCard.tsx"),
    "iOS Session client": ("0.6.0", "apps/interaction-ios/InteractionCompanion/Services/SessionClient.swift"),
}
present = {k: v for k, v in landed.items() if os.path.exists(v[1])}
if present:
    need("在落地前不會出現在這裡" not in unreleased,
         "CHANGELOG 最上層段仍寫『尚未落地的項目…在落地前不會出現在這裡』，"
         "但這些已在 HEAD 上：%s" % sorted(present))
    for name, (version, path) in present.items():
        base = os.path.basename(path)
        section = version_section(ch, version)
        need(section != "", "CHANGELOG 缺少落地版本段 `## [%s]`" % version)
        need(base in section or name in section,
             "CHANGELOG `## [%s]` 段沒有 %s（%s）的條目，但程式碼在該版本落地" % (version, name, base))

# ---- evidence-honesty-013：ARCHITECTURE.md ---------------------------------
arch = read("docs/ARCHITECTURE.md")
sync_landed = "CHARACTER_SYNC_PROJECTION" in read("apps/interaction-desktop/src/statusProjection.ts")
ios_landed = os.path.exists("apps/interaction-ios/InteractionCompanion/Services/SessionClient.swift")
if sync_landed or ios_landed:
    need("搜尋不到對應實作" not in arch,
         "docs/ARCHITECTURE.md 仍以『搜尋不到對應實作』宣告桌面同步 UI／iOS Session client 未落地"
         "（statusProjection=%s SessionClient.swift=%s）" % (sync_landed, ios_landed))
if sync_landed:
    need("尚未出現在 `statusProjection.ts`" not in arch,
         "docs/ARCHITECTURE.md 仍寫同步人話『尚未出現在 statusProjection.ts』，"
         "但 CHARACTER_SYNC_PROJECTION 已存在")

# ---- evidence-honesty-014：threat-model.md 的 gate() 引用 -------------------
tm = read("docs/aip/threat-model.md")
sess = read("crates/interaction-session/src/session.rs")
sec3 = tm.split("## 3.", 1)[1].split("\n## 4.", 1)[0] if "## 3." in tm else tm
need(not re.search(r"`:\d+", sec3),
     "docs/aip/threat-model.md §3 仍用硬編行號（`:NNN`）引用 session.rs::gate() 的每一步；"
     "HEAD 上 gate() 已漂移，請改引用函式名／步驟註解字串")
need("fn gate(" in sess, "session.rs 找不到 `fn gate(`（threat-model.md 的錨點失效）")
for step in range(1, 14):
    need("// %d." % step in sess,
         "session.rs 找不到 gate() 第 %d 步的註解錨點 `// %d.`" % (step, step))

# ---- evidence-honesty-016：EvidenceClass 有沒有消費端 ----------------------
consumers = []
for root in ("crates/interaction-runtime/src", "crates/interaction-api/src", "crates/interaction-cli/src"):
    for dirpath, _dirs, files in os.walk(root):
        for f in sorted(files):
            if f.endswith(".rs") and re.search(r"EvidenceClass|evidence_class",
                                               read(os.path.join(dirpath, f))):
                consumers.append(os.path.join(dirpath, f))
aip = read("docs/aip/README.md")
sec10 = aip.split("## 10.", 1)[1].split("\n## 11.", 1)[0] if "## 10." in aip else ""
if not consumers:
    need("尚未接進" in sec10 or "implemented-unverified" in sec10,
         "docs/aip/README.md §10 把 EvidenceClass 說成已『用於 diagnostics』，"
         "但 runtime／api／cli 沒有任何生產或消費端；措辭必須誠實標明尚未接進")
else:
    need("尚未接進" not in sec10,
         "docs/aip/README.md §10 說 EvidenceClass 尚未接進，但已有消費端：%s" % consumers)

# ---- release-provenance-074/075：完整性宣稱 --------------------------------
sm = read("crates/interaction-cli/src/selfmgmt.rs")
tail = sm.split("skipping verification", 1)[1][:200] if "skipping verification" in sm else ""
need("Ok(())" not in tail,
     "selfmgmt.rs 的 verify_checksum 仍是 fail-open（缺 .sha256 就照裝），"
     "但 README／INSTALL／QUICKSTART 宣稱 sha256 驗證")
install = read("docs/INSTALL.md")
need("未簽章" in install and ("SBOM" in install or "provenance" in install),
     "docs/INSTALL.md 沒有誠實寫明桌面安裝包未簽章、無 SBOM／provenance")
escape = "INTERACT_AI_ALLOW_UNVERIFIED_DOWNLOAD"
if escape in sm:
    need(escape in install, "selfmgmt.rs 有 %s 逃生門，但 docs/INSTALL.md 沒有記載" % escape)

# ---- release-provenance-080：平台覆蓋宣稱 ----------------------------------
targets = set(re.findall(r"target:\s*([a-z0-9_]+-[a-z0-9-]+)", read(".github/workflows/release.yml")))
if "aarch64-unknown-linux-gnu" not in targets:
    need("aarch64-unknown-linux-gnu" in install,
         "release.yml 不建置 Linux aarch64，docs/INSTALL.md 必須寫明該平台需從原始碼編譯")


# ---- evidence-index：已發布版本的 canonical 事實 -----------------------------
import json, subprocess
idx_path = "docs/releases/evidence-index.json"
try:
    idx = json.loads(read(idx_path))
except Exception as e:  # noqa: BLE001
    idx = None
    need(False, "%s 不是合法 JSON：%s" % (idx_path, e))
if idx:
    need(idx.get("schemaVersion") == 1, "%s schemaVersion 必須是 1" % idx_path)
    stale_docs = ["README.md", "docs/ARCHITECTURE.md", "docs/FEATURES.md", "CLAUDE.md", "AGENTS.md"]
    stale_text = {d: read(d) for d in stale_docs if os.path.exists(d)}
    for rel in idx.get("releases", []):
        tag = rel.get("tag", "")
        commit = rel.get("commit", "")
        try:
            resolved = subprocess.run(["git", "rev-list", "-n1", tag], capture_output=True, text=True, check=False).stdout.strip()
        except OSError:
            resolved = ""
        need(resolved != "", "evidence-index：tag %s 在這個 repo 裡不存在" % tag)
        need(resolved == "" or resolved == commit,
             "evidence-index：tag %s 指向 %s，索引寫 %s" % (tag, resolved[:12], commit[:12]))
        for key, path in rel.get("docs", {}).items():
            need(os.path.exists(path), "evidence-index：%s 的 docs.%s 指向不存在的 %s" % (tag, key, path))
        # 已發布的版本不得在總覽文件裡仍寫成「尚未 tag／發布」「候選」「進行中／開發中」。
        # 對抗審查 427c806 指出只比對三個固定片語會被同義措辭繞過（ARCHITECTURE.md 仍有
        # 「v0.6.0 進行中」）：改成一組同義詞，任何一個與已發布版本字串同行就擋。
        ver = rel.get("version", "")
        stale_words = ("尚未 tag", "尚未發布", "尚未落地", "候選版本", "候選", "進行中", "開發中", "未發布")
        for d, text in stale_text.items():
            for line in text.splitlines():
                if ("v" + ver) in line:
                    hit = [w for w in stale_words if w in line]
                    need(not hit,
                         "%s 仍把已發布的 v%s 寫成%s：%s" % (d, ver, "／".join(hit), line.strip()[:80]))

for f in fails:
    print("  ✘ " + f)
print()
print("docs-claims: %d passed / %d failed" % (checks - len(fails), len(fails)))
sys.exit(1 if fails else 0)
PY
