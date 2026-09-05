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
#   - deprecation-ledger.md 表頭自報的條數／完整七欄表格數必須等於實際的節數（自我計數不得漂移）
#   - evidence-index.json 的 candidates[] 最小 schema：必填欄位、branch／baseCommit 是真的 ref、
#     progress 與 docs.* 指到存在的檔、進度檔在 baseCommit..HEAD 之間被更新過
#   - CHANGELOG [Unreleased] 的 Known limitations 不得掛著已經被修掉的限制
#     （限制文字 ＋『修掉它的測試存不存在』兩者同時成立就紅；劃掉 `~~…~~` 不算）
#   - 文件引用的「`<test>.rs` N 測」必須等於那支檔案裡真的有幾支 `#[test]`／`#[tokio::test]`
#   - AGENTS.md §7 指到的進度文件必須存在，且它標的 Blockers 章節號與檔案裡的真實節號一致
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


# ---- deprecation-ledger：自我計數不得漂移 ----------------------------------
# 這份表的表頭自己報「本表登記 N 條、其中 M 條有完整七欄表格」。這種手抄數字加一條就會
# 過期，而過期的方式很安靜：表看起來仍然完整，只是少算了一條沒有人再檢查的相容路徑。
ledger_path = "docs/aip/deprecation-ledger.md"
if os.path.exists(ledger_path):
    ledger = read(ledger_path)
    LEDGER_FIELDS = ["為什麼存在", "適用版本", "移除前需要的證據", "資料遷移", "回退方式", "下一檢查里程碑", "owner"]
    sections = re.split(r"^### ", ledger, flags=re.M)[1:]
    full = [
        s for s in sections
        if all(re.search(r"^\|\s*%s\s*\|" % re.escape(f), s, flags=re.M) for f in LEDGER_FIELDS)
    ]
    m_total = re.search(r"本表登記\s*\*\*(\d+)\s*條\*\*", ledger)
    m_full = re.search(r"其中\s*(\d+)\s*條有完整七欄表格", ledger)
    need(m_total is not None, "%s 的表頭找不到「本表登記 **N 條**」的自我計數" % ledger_path)
    need(m_full is not None, "%s 的表頭找不到「其中 N 條有完整七欄表格」的自我計數" % ledger_path)
    if m_total:
        need(int(m_total.group(1)) == len(sections),
             "%s 表頭寫「本表登記 %s 條」，實際有 %d 個 `### ` 節"
             % (ledger_path, m_total.group(1), len(sections)))
    if m_full:
        need(int(m_full.group(1)) == len(full),
             "%s 表頭寫「其中 %s 條有完整七欄表格」，實際有 %d 節帶著七個欄位齊全的表格"
             % (ledger_path, m_full.group(1), len(full)))

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
        # 對抗審查 713f8fe 指出這一段比它自己的註解弱兩級：(1) 只認 `v` 前綴，「0.6.0 目前仍在
        # 候選階段」抓不到；(2) 只比對同一行，版本與過期詞被硬折行分開就漏掉；(3) 少了這個 repo
        # 自己在用的「準備中」（release-prepare 留下的形狀就是 `## [0.5.0] — …（準備中）`）。
        stale_words = ("尚未 tag", "尚未 Tag", "尚未發布", "尚未落地", "尚未完成", "候選版本", "候選",
                       "進行中", "開發中", "未發布", "準備中", "草稿", "pre-release", "release candidate")
        ver_pat = re.compile(r"(?<![0-9.])v?" + re.escape(ver) + r"(?![0-9.])")
        for d, text in stale_text.items():
            lines = text.splitlines()
            for i, line in enumerate(lines):
                if not ver_pat.search(line):
                    continue
                # 版本字串所在的「段落窗」：前後各一行，硬折行才不會變成漏網之魚。
                # 但相鄰行若自己提到**別的**版本（例如下一行在講 v0.7.0 候選），那句過期詞
                # 是講那個版本的，不算在這個版本頭上——否則這支 lint 會開始誤殺。
                other_ver = re.compile(r"v?\d+\.\d+\.\d+")
                window_lines = [
                    l for l in lines[max(0, i - 1):i + 2]
                    if l is line or not [t for t in other_ver.findall(l) if t.lstrip("v") != ver]
                ]
                window = "\n".join(window_lines)
                hit = [w for w in stale_words if w in window]
                need(not hit,
                     "%s:%d 仍把已發布的 v%s 寫成%s：%s"
                     % (d, i + 1, ver, "／".join(hit), line.strip()[:80]))
    # ---- candidates[]：候選分支的最小 schema（過去完全沒有被 lint 看過）--------
    # 已發布那半部由上面的 tag→commit 對帳守住；候選這半部原本零覆蓋，於是它可以
    # 一邊寫著「未決定號碼」，一邊讓 26 個 v0.7.0 戳記散進契約文件而沒有人紅。
    CANDIDATE_KEYS = ("branch", "baseCommit", "status", "versionPolicy", "progress", "docs")
    def git_out(args):
        try:
            r = subprocess.run(args, capture_output=True, text=True, check=False)
            return r.stdout.strip() if r.returncode == 0 else ""
        except OSError:
            return ""
    for i, cand in enumerate(idx.get("candidates", [])):
        label = "%s candidates[%d]" % (idx_path, i)
        for key in CANDIDATE_KEYS:
            need(key in cand and cand[key], "%s 缺少必要欄位 `%s`" % (label, key))
        branch = cand.get("branch", "")
        if branch:
            need(git_out(["git", "rev-parse", "--verify", "--quiet", branch + "^{commit}"]) != "",
                 "%s 的 branch `%s` 在這個 repo 裡不是一個 ref" % (label, branch))
        base = cand.get("baseCommit", "")
        if base:
            resolved = git_out(["git", "rev-parse", "--verify", "--quiet", base + "^{commit}"])
            need(resolved != "", "%s 的 baseCommit %s 在這個 repo 裡不存在" % (label, base[:12]))
        progress = cand.get("progress", "")
        if progress:
            need(os.path.exists(progress),
                 "%s 的 progress 指向不存在的 %s" % (label, progress))
        for key, path in (cand.get("docs") or {}).items():
            need(os.path.exists(path), "%s 的 docs.%s 指向不存在的 %s" % (label, key, path))
        # 「號碼未定」與「契約文件裡蓋滿版本戳記」不能同時成立——本輪就是這樣漂掉的：
        # candidates 說「未決定號碼」，同一棵樹的 docs/aip/*.md 卻已經有 26 個 v0.7.0。
        if "未決定號碼" in cand.get("versionPolicy", ""):
            stamped = []
            for f in sorted(os.listdir("docs/aip")) if os.path.isdir("docs/aip") else []:
                if f.endswith(".md") and re.search(r"v\d+\.\d+\.\d+", read(os.path.join("docs/aip", f))):
                    stamped.append("docs/aip/" + f)
            need(not stamped,
                 "%s 的 versionPolicy 寫「未決定號碼」，但這些契約文件已經蓋上具體版本戳記：%s"
                 % (label, stamped[:5]))
        # 「工作落地了、追蹤文件卻沒動」：候選分支的進度檔必須在 baseCommit..HEAD 之間被改過。
        if base and progress and os.path.exists(progress):
            is_ancestor = subprocess.run(["git", "merge-base", "--is-ancestor", base, "HEAD"],
                                         capture_output=True, text=True, check=False).returncode == 0
            if is_ancestor:
                touched = git_out(["git", "log", "--oneline", "%s..HEAD" % base, "--", progress])
                need(touched != "",
                     "%s：%s..HEAD 有提交，但 progress `%s` 一次都沒被更新"
                     % (label, base[:12], progress))

# ---- CHANGELOG [Unreleased] Known limitations：修掉的限制不得繼續掛著 --------
# 這一段的標題自己寫著「修掉時同步刪除」，但原本沒有任何機械檢查，所以「已經被本分支
# 修掉的限制」可以原封不動地留著，而 lint 仍然全綠。每一條 = 一句限制文字 ＋ 一個
# 「它已經被修掉」的可執行證據；證據成立時那句話就不得再出現。
# 劃掉的（`~~…~~`）不算——那正是「修掉時同步刪除」的正確寫法。
def strip_struck(s):
    return re.sub(r"~~.*?~~", "", s, flags=re.S)

def file_has(path, needle):
    return os.path.exists(path) and needle in read(path)

def src_has(needle, roots=("crates",)):
    for root in roots:
        for dirpath, _dirs, files in os.walk(root):
            if os.sep + "tests" in dirpath + os.sep:
                continue
            for f in files:
                if f.endswith(".rs") and needle in read(os.path.join(dirpath, f)):
                    return True
    return False

kl_blocks = []
for m in re.finditer(r"^#{3,4} Known limitations[^\n]*\n", unreleased, flags=re.M):
    rest = unreleased[m.end():]
    nxt = re.search(r"^#{2,4} ", rest, flags=re.M)
    kl_blocks.append(rest[:nxt.start()] if nxt else rest)
kl_text = strip_struck("\n".join(kl_blocks))
# 文件是硬折行的，一句話常被切成兩行；比對前把空白抽掉，換行才不會變成漏網之魚。
kl_flat = re.sub(r"\s+", "", kl_text)

DECL_LOOP = "crates/interaction-runtime/tests/declarative_session_loop.rs"
rebind_fixed = (file_has(DECL_LOOP, "reenable_rebinds_without_restart")
                and not src_has("needs-restart-to-rebind"))
frag_landed = (os.path.exists("crates/interaction-adapter-declarative/src/fragment.rs")
               and file_has(DECL_LOOP, "a_fragmenting_device_receives_the_snapshot_and_is_a_full_state_member"))
KNOWN_LIMITATION_PROBES = [
    ("仍需重啟 daemon 才重新綁定", rebind_fixed,
     "`declarative_session_loop.rs::reenable_rebinds_without_restart` 存在，且 production code 已無 `needs-restart-to-rebind`"),
    ("需重啟才重新綁定", rebind_fixed,
     "`declarative_session_loop.rs::reenable_rebinds_without_restart` 存在，且 production code 已無 `needs-restart-to-rebind`"),
    ("分段／per-member diff／縮減 profile 是協定層決定，本分支未做", frag_landed,
     "`adapter-declarative/src/fragment.rs` 與 `a_fragmenting_device_receives_the_snapshot_and_is_a_full_state_member` 都在 HEAD 上"),
    ("沒有測試擋得住「選填欄位寫成 `null` 進 canonical」",
     file_has("crates/interaction-session/tests/state_semantics.rs", "semantic_state_never_serializes_a_null"),
     "`state_semantics.rs::semantic_state_never_serializes_a_null` 存在"),
]
if kl_blocks:
    for phrase, fixed, why in KNOWN_LIMITATION_PROBES:
        need(not (fixed and re.sub(r"\s+", "", phrase) in kl_flat),
             "CHANGELOG `[Unreleased]` 的 Known limitations 還掛著「%s」，但它已經被修掉（%s）；"
             "照那一段標題的規矩，修掉時要同步刪除或劃掉" % (phrase, why))
else:
    need(False, "CHANGELOG `[Unreleased]` 找不到任何 `### Known limitations` 小節")

# ---- 文件引用的「N 測」必須等於那支測試檔真的有幾支 -------------------------
# 同一支 declarative_session_loop.rs 在四份文件裡有過 7／13／25／26 四個數字。
# 手抄的計數會安靜地過期，讀者無法判斷哪一個是真的，所以改由檔案本身當唯一來源。
rs_tests = {}
for crate in sorted(os.listdir("crates")) if os.path.isdir("crates") else []:
    tdir = os.path.join("crates", crate, "tests")
    if not os.path.isdir(tdir):
        continue
    for f in sorted(os.listdir(tdir)):
        if f.endswith(".rs"):
            rs_tests.setdefault(f, []).append(os.path.join(tdir, f))

def live_test_count(path):
    return len(re.findall(r"#\[tokio::test|#\[test\]", read(path)))

COUNT_DOCS = []
for d in ("docs/aip", "docs/releases"):
    if os.path.isdir(d):
        for f in sorted(os.listdir(d)):
            if f.endswith(".md") and (d == "docs/aip" or f.startswith("v0.7.0-")):
                COUNT_DOCS.append(os.path.join(d, f))
COUNT_DOCS.append("docs/acceptance-evidence.md")
# `**26 測**` 這種粗體寫法也要算進來——否則作者只要把數字加粗，這個把關就會安靜地失效。
count_pat = re.compile(r"`([A-Za-z0-9_./-]*?([A-Za-z0-9_]+\.rs))`\s*[（(]?\s*\**\s*(\d+)\s*測")
for doc in COUNT_DOCS:
    if not os.path.exists(doc):
        continue
    for lineno, line in enumerate(read(doc).splitlines(), 1):
        for _full, base, claimed in count_pat.findall(line):
            paths = rs_tests.get(base, [])
            if len(paths) != 1:
                continue  # 檔名不唯一（或不是 crates/*/tests 下的整合測試）就不猜
            actual = live_test_count(paths[0])
            need(int(claimed) == actual,
                 "%s:%d 寫 `%s` %s 測，實際 %d 支（`#[test]`＋`#[tokio::test]`）"
                 % (doc, lineno, base, claimed, actual))

# ---- AGENTS.md §7：接續進度指到的檔案與章節必須真的存在 ---------------------
# 「先讀 §4（Blockers）」這種指路只要進度檔重新編號就會安靜地指錯，而它正是新 session
# 的唯一入口。規則：§7 每一個指到進度檔的 bullet 都要標明該檔 Blockers 的章節號。
agents = read("AGENTS.md") if os.path.exists("AGENTS.md") else ""
if "## 7." in agents:
    sec7 = agents.split("## 7.", 1)[1].split("\n## ", 1)[0]
    # bullet 是硬折行的，先把每一個 `- ` 開頭的邏輯項目併回一行再比對。
    bullets = [re.sub(r"\s+", " ", b) for b in re.split(r"^- ", sec7, flags=re.M)[1:]
               if "progress.md" in b]
    need(bullets != [], "AGENTS.md §7 沒有指向任何進度文件")
    for line in bullets:
        for path in re.findall(r"`(docs/releases/[A-Za-z0-9_.\-]*progress\.md)`", line):
            need(os.path.exists(path), "AGENTS.md §7 指向不存在的 %s" % path)
            if not os.path.exists(path):
                continue
            m = re.search(r"^## (\d+)\. Blockers", read(path), flags=re.M)
            need(m is not None, "%s 沒有 `## N. Blockers` 章節，AGENTS.md §7 卻叫人先讀 Blockers" % path)
            cited = re.search(r"Blockers[^§\n]{0,12}§(\d+)", line)
            need(cited is not None,
                 "AGENTS.md §7 指向 %s 卻沒寫它的 Blockers 在第幾節（章節一改號就會指錯）" % path)
            if m and cited:
                need(cited.group(1) == m.group(1),
                     "AGENTS.md §7 寫 %s 的 Blockers 在 §%s，實際在 §%s"
                     % (path, cited.group(1), m.group(1)))


for f in fails:
    print("  ✘ " + f)
print()
print("docs-claims: %d passed / %d failed" % (checks - len(fails), len(fails)))
sys.exit(1 if fails else 0)
PY
