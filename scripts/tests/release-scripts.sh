#!/usr/bin/env bash
# 發布腳本／workflow 的單元測試（無網路、無 daemon、不碰真 repo 狀態）。
#
#   bash scripts/tests/release-scripts.sh
#
# 覆蓋：
#   - 四支 release 腳本、get.sh、scripts/tests/*.sh（含 architecture-checks.sh）、
#     scripts/tauri-ax-walkthrough.sh 與 scripts/drills/*.sh 的 `bash -n` 語法檢查
#   - bash 3.2（macOS 預設 /bin/bash）對 `set -u` 下空陣列展開的相容性（release-provenance-071）
#   - release-verify.sh 跳過關卡時必須誠實輸出「跳過」，且收尾不得寫 all gates passed（release-provenance-073）
#   - CI 必需 check 清單：缺席的 job 必須讓關卡失敗（release-provenance-077）
#   - release.yml：先 draft → 建置 → gate → finalize，桌面 bundle 有 .sha256（071/072/075/079）
#   - 安裝器／CLI 宣稱的平台不得超出 release.yml 實際建置的 target（release-provenance-080）
#   - get.sh 缺 .sha256 時 fail-closed，且不把未驗證的二進位裝進 bin-dir（release-provenance-074）
#   - release.yml 內嵌 shell／python 真的可執行：finalize 缺資產會擋、desktop 會算 .sha256（075/079）
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
PASS=0; FAIL=0
ok()   { echo "  ✔ $1"; PASS=$((PASS + 1)); }
bad()  { echo "  ✘ $1${2:+ — $2}"; FAIL=$((FAIL + 1)); }
check(){ if [[ "$2" == "0" ]]; then ok "$1"; else bad "$1" "${3:-}"; fi; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "release-scripts tests @ $ROOT"

# ---------------------------------------------------------------- 語法檢查
SYNTAX_FILES=(scripts/release.sh scripts/release-prepare.sh scripts/release-verify.sh
              scripts/release-tag.sh scripts/ci-required-checks.sh scripts/get.sh
              scripts/tests/release-scripts.sh scripts/tests/docs-claims.sh
              scripts/tests/architecture-checks.sh scripts/tauri-ax-walkthrough.sh)
# 演練腳本（scripts/drills/*.sh）沒有 CI，語法檢查是它們唯一的自動化把關；
# 空目錄不是「沒東西要檢查」，是演練不見了。
DRILL_SCRIPTS=()
while IFS= read -r f; do DRILL_SCRIPTS+=("$f"); done < <(ls scripts/drills/*.sh 2>/dev/null | sort)
if [[ "${#DRILL_SCRIPTS[@]}" -gt 0 ]]; then
  ok "scripts/drills/ 有 ${#DRILL_SCRIPTS[@]} 支演練腳本"
  SYNTAX_FILES+=("${DRILL_SCRIPTS[@]}")
else
  bad "scripts/drills/*.sh" "一支演練腳本都沒有"
fi
for f in "${SYNTAX_FILES[@]}"; do
  if [[ -f "$f" ]]; then
    /bin/bash -n "$f" 2>"$WORK/nerr"; check "bash -n $f" $? "$(head -1 "$WORK/nerr")"
  else
    bad "bash -n $f" "檔案不存在"
  fi
done

# --------------------------------------- 071：set -u 下的空陣列展開必須被保護
# bash 3.2 對 `"${A[@]}"`（A 為空陣列）會以 unbound variable 中止。
# `${A[@]+"${A[@]}"}` 是已保護的寫法（含 `[@]+`），不算未保護。
UNGUARDED=$(grep -nE '"\$\{[A-Za-z_][A-Za-z0-9_]*\[@\]\}"' \
  scripts/release.sh scripts/release-prepare.sh scripts/release-verify.sh scripts/release-tag.sh 2>/dev/null \
  | grep -v '\[@\]+' || true)
if [[ -z "$UNGUARDED" ]]; then
  ok "release 腳本沒有未保護的 \${ARR[@]} 展開（bash 3.2 相容）"
else
  bad "release 腳本有未保護的 \${ARR[@]} 展開（bash 3.2 會 unbound variable）" "$(echo "$UNGUARDED" | tr '\n' ' ')"
fi

# 行為測試：用 /bin/bash（macOS 上是 3.2）跑 release-tag.sh 的「不跳過 CI」路徑。
TAGDIR="$WORK/tagrepo"
mkdir -p "$TAGDIR/scripts" "$TAGDIR/bin"
cp scripts/release-tag.sh "$TAGDIR/scripts/"
cat > "$TAGDIR/scripts/release-verify.sh" <<'STUB'
#!/usr/bin/env bash
echo "VERIFY-CALLED $*" >> "$STUB_LOG"
exit 0
STUB
chmod +x "$TAGDIR/scripts/release-verify.sh"
cat > "$TAGDIR/bin/git" <<'STUB'
#!/usr/bin/env bash
case "$1 ${2:-}" in
  "fetch -q") exit 0 ;;
  "rev-parse HEAD") echo "1111111111111111111111111111111111111111"; exit 0 ;;
  "branch --show-current") echo "main"; exit 0 ;;
  "merge-base --is-ancestor") exit 0 ;;
  "tag -a") echo "GIT-TAG $*" >> "$STUB_LOG"; exit 0 ;;
  "push origin") echo "GIT-PUSH $*" >> "$STUB_LOG"; exit 0 ;;
esac
case "$1" in
  rev-parse) echo "1111111111111111111111111111111111111111"; exit 0 ;;
  *) exit 0 ;;
esac
STUB
chmod +x "$TAGDIR/bin/git"

run_tag() { # run_tag <log> [flags…]
  local log="$1"; shift
  : > "$log"
  ( cd "$TAGDIR" && STUB_LOG="$log" PATH="$TAGDIR/bin:$PATH" \
      /bin/bash scripts/release-tag.sh 9.9.9 --allow-branch "$@" >"$log.out" 2>&1 )
  return $?
}

run_tag "$WORK/tag-normal.log"; RC=$?
check "release-tag.sh（不帶 --skip-ci）在 /bin/bash $(/bin/bash -c 'echo ${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}') 下不會 crash" \
  "$([[ "$RC" == 0 ]] && echo 0 || echo 1)" "rc=$RC $(head -3 "$WORK/tag-normal.log.out" | tr '\n' ' ')"
grep -q "VERIFY-CALLED 9.9.9" "$WORK/tag-normal.log"; \
  check "release-tag.sh（不帶 --skip-ci）真的呼叫 release-verify.sh" $?
grep -q "GIT-TAG" "$WORK/tag-normal.log"; check "release-tag.sh 在 verify 通過後建立 tag" $?

run_tag "$WORK/tag-skip.log" --skip-ci; RC=$?
check "release-tag.sh --skip-ci 也仍可執行" "$([[ "$RC" == 0 ]] && echo 0 || echo 1)" "rc=$RC"
grep -q -- "--skip-ci" "$WORK/tag-skip.log"; check "release-tag.sh --skip-ci 把旗標轉給 release-verify.sh" $?

# --------------------------------------- 073：跳過的關卡必須誠實標示「跳過」
VDIR="$WORK/verifyrepo"
mkdir -p "$VDIR/scripts" "$VDIR/bin" "$VDIR/apps/interaction-desktop/src-tauri" "$VDIR/schemas"
cp scripts/release-verify.sh "$VDIR/scripts/"
[[ -f scripts/ci-required-checks.sh ]] && cp scripts/ci-required-checks.sh "$VDIR/scripts/"
printf 'version = "9.9.9"\n' > "$VDIR/Cargo.toml"
printf 'version = "9.9.9"\n' > "$VDIR/apps/interaction-desktop/src-tauri/Cargo.toml"
printf '{"version": "9.9.9"}\n' > "$VDIR/apps/interaction-desktop/src-tauri/tauri.conf.json"
printf '{"version": "9.9.9"}\n' > "$VDIR/apps/interaction-desktop/package.json"
printf '# Changelog\n\n## [9.9.9] - 2999-01-01\n\n- something real\n' > "$VDIR/CHANGELOG.md"
printf '{"info": {"version": "9.9.9"}}\n' > "$VDIR/schemas/openapi.json"
cat > "$VDIR/bin/git" <<'STUB'
#!/usr/bin/env bash
case "$*" in
  "status --porcelain") exit 0 ;;
  "diff --check") exit 0 ;;
  "rev-parse --short HEAD") echo "abc1234"; exit 0 ;;
  "rev-parse HEAD") echo "1111111111111111111111111111111111111111"; exit 0 ;;
  "rev-parse v9.9.9") echo "unknown revision" >&2; exit 128 ;;
  grep*) exit 1 ;;
  *) exit 0 ;;
esac
STUB
chmod +x "$VDIR/bin/git"
( cd "$VDIR" && PATH="$VDIR/bin:$PATH" /bin/bash scripts/release-verify.sh 9.9.9 --skip-ci ) \
  > "$WORK/verify.out" 2>&1
VRC=$?
check "release-verify.sh --skip-ci 在結構關卡全過時 exit 0" "$([[ "$VRC" == 0 ]] && echo 0 || echo 1)" \
  "rc=$VRC $(tail -2 "$WORK/verify.out" | tr '\n' ' ')"
grep -qiE '(SKIP|跳過)' "$WORK/verify.out"
check "release-verify.sh --skip-ci 明確輸出『跳過』（CI 關卡未評估）" $? "$(tr '\n' '|' < "$WORK/verify.out")"
grep -qiE '(SKIP|跳過).*(test|測試)' "$WORK/verify.out"
check "release-verify.sh 未帶 --run-tests 時明確輸出『測試未跑』" $?

# --run-tests 必須涵蓋 Tauri backend（workspace exclude 的 leaf crate；CHANGELOG claim-check 住在那裡）
# 與 AIP codegen 漂移：v0.6.0 發布後 ea7de59 在本機通過、到 CI 的 Tauri backend job 才變紅，
# 就是因為本機關卡漏了這一段。這裡靜態核對 --run-tests 區塊真的呼叫了它們（每段各自取 $?，不經管線）。
RT="$(awk '/RUN_TESTS" == "1"/,/^else/' scripts/release-verify.sh)"
grep -q 'cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml' <<<"$RT"
check "release-verify.sh --run-tests 跑 src-tauri 的 cargo test（CHANGELOG claim-check）" $?
grep -q 'cargo clippy --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml' <<<"$RT"
check "release-verify.sh --run-tests 跑 src-tauri 的 clippy" $?
grep -q 'pnpm aip:check' <<<"$RT"
check "release-verify.sh --run-tests 跑 pnpm aip:check（codegen 漂移）" $?
grep -Eq '\| *(tail|grep|tee)' <<<"$RT" && P=1 || P=0
check "release-verify.sh --run-tests 區塊沒有會吞掉退出碼的管線（tail/grep/tee）" "$P"
if grep -qE '^✔ all gates passed for v9\.9\.9$' "$WORK/verify.out"; then
  bad "有關卡被跳過時，收尾不得是無限定詞的『all gates passed』" "$(grep '^✔' "$WORK/verify.out" | tail -1)"
else
  ok "有關卡被跳過時，收尾不是無限定詞的『all gates passed』"
fi
grep -qiE 'passed-with-skips|passed .*skipped' "$WORK/verify.out"
check "release-verify.sh 收尾寫出 passed-with-skips（誠實階梯）" $? "$(tail -1 "$WORK/verify.out")"

# --------------------------------------- 077：必需 check 缺席就要失敗
if [[ -x scripts/ci-required-checks.sh || -f scripts/ci-required-checks.sh ]]; then
  REQ=$(python3 - <<'PY'
import re
src = open(".github/workflows/ci.yml").read()
jobs = src.split("\njobs:", 1)[1]
names = []
cur = None
for line in jobs.splitlines():
    m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
    if m:
        cur = m.group(1); names.append([cur, cur]); continue
    m = re.match(r'^    name:\s*(.+?)\s*$', line)
    if m and names:
        names[-1][1] = m.group(1).strip().strip(chr(34)).strip(chr(39))
print("\n".join(n[1] for n in names))
PY
)
  ALLGREEN=$(echo "$REQ" | sed 's/$/=success/')
  echo "$ALLGREEN" | /bin/bash scripts/ci-required-checks.sh >"$WORK/req-all.out" 2>&1
  check "ci-required-checks：四個必需 job 全 success → 通過" $? "$(cat "$WORK/req-all.out")"

  MISSING=$(echo "$ALLGREEN" | grep -v -i 'e2e')
  echo "$MISSING" | /bin/bash scripts/ci-required-checks.sh >"$WORK/req-missing.out" 2>&1
  RC=$?
  check "ci-required-checks：e2e 缺席 → 失敗（不得報 all success）" "$([[ "$RC" != 0 ]] && echo 0 || echo 1)" \
    "rc=$RC $(cat "$WORK/req-missing.out")"

  NOTGREEN=$(echo "$ALLGREEN" | sed '1s/=success/=failure/')
  echo "$NOTGREEN" | /bin/bash scripts/ci-required-checks.sh >"$WORK/req-red.out" 2>&1
  RC=$?
  check "ci-required-checks：有 job 非 success → 失敗" "$([[ "$RC" != 0 ]] && echo 0 || echo 1)" "rc=$RC"

  echo "" | /bin/bash scripts/ci-required-checks.sh >"$WORK/req-empty.out" 2>&1
  RC=$?
  check "ci-required-checks：完全沒有 check-run → 失敗（fail-closed）" "$([[ "$RC" != 0 ]] && echo 0 || echo 1)" "rc=$RC"
else
  bad "scripts/ci-required-checks.sh 不存在" "CI 關卡沒有必需 job 清單"
fi

grep -q -- "--paginate" scripts/release-verify.sh
check "release-verify.sh 的 gh api 有 --paginate（check-run 超過 30 筆不得靜默截斷）" $?

# --------------------------------------- workflow YAML 與結構（071/072/075/079/080）
python3 - <<'PY' > "$WORK/yaml.out" 2>&1
import sys, re
try:
    import yaml
except ImportError:
    print("SKIP no-pyyaml"); sys.exit(0)
fails = []
for p in (".github/workflows/ci.yml", ".github/workflows/release.yml"):
    try:
        yaml.safe_load(open(p))
    except Exception as e:
        fails.append(f"{p}: {e}")
rel = yaml.safe_load(open(".github/workflows/release.yml"))
jobs = rel.get("jobs", {})
raw = open(".github/workflows/release.yml").read()

# 072：必須有一個 gate job 在建置前查被 tag 的 commit 的 CI check-runs
gate = [n for n, j in jobs.items() if "check-run" in yaml.dump(j) or "ci-required-checks" in yaml.dump(j)]
if not gate:
    fails.append("release.yml 沒有任何 job 查 CI check-runs（tag→Release 沒有 CI 關卡）")
else:
    for n in ("cli", "desktop", "extras", "create-release"):
        needs = jobs.get(n, {}).get("needs", [])
        needs = [needs] if isinstance(needs, str) else list(needs or [])
        if not any(g in needs for g in gate) and n != "create-release":
            fails.append(f"release.yml job {n} 沒有 needs 到 CI gate {gate}")

# 079：先 draft，最後才 publish
cr = yaml.dump(jobs.get("create-release", {}))
if "--draft" not in cr:
    fails.append("release.yml create-release 沒有以 --draft 建立（建置中就公開）")
fin = [n for n, j in jobs.items()
       if "--draft=false" in yaml.dump(j) or "draft: false" in yaml.dump(j).replace("releaseDraft", "draft")]
fin = [n for n, j in jobs.items() if "--draft=false" in yaml.dump(j)]
if not fin:
    fails.append("release.yml 沒有 finalize job 把 draft 轉為已發布")
else:
    needs = jobs[fin[0]].get("needs", [])
    needs = [needs] if isinstance(needs, str) else list(needs or [])
    for n in ("cli", "desktop", "extras"):
        if n not in needs:
            fails.append(f"finalize job {fin[0]} 沒有 needs: {n}")
    if "assets" not in yaml.dump(jobs[fin[0]]):
        fails.append(f"finalize job {fin[0]} 沒有檢查資產是否齊全")

# 075：桌面 bundle 必須產生並上傳 .sha256
desktop = yaml.dump(jobs.get("desktop", {}))
if ".sha256" not in desktop:
    fails.append("release.yml desktop job 沒有為 bundle 產生／上傳 .sha256")
if "releaseDraft: false" in raw:
    fails.append("release.yml 仍有 releaseDraft: false（tauri-action 會在建置中公開 release）")

# 080：安裝器／CLI 宣稱的平台不得超出 matrix
targets = set()
for inc in jobs.get("cli", {}).get("strategy", {}).get("matrix", {}).get("include", []):
    targets.add(inc["target"])
getsh = open("scripts/get.sh").read()
claimed = set(re.findall(r'TRIPLE="([a-z0-9_]+-[a-z0-9-]+)"', getsh))
extra = claimed - targets
if extra:
    fails.append(f"scripts/get.sh 宣稱支援但 release.yml 未建置的 target: {sorted(extra)}")
sm = open("crates/interaction-cli/src/selfmgmt.rs").read()
cli_claimed = set(re.findall(r'=> Ok\("([a-z0-9_]+-[a-z0-9-]+)"\)', sm))
extra = cli_claimed - targets
if extra:
    fails.append(f"selfmgmt.rs target_triple() 宣稱支援但 release.yml 未建置的 target: {sorted(extra)}")

for f in fails:
    print("FAIL", f)
print("DONE")
PY
if grep -q "SKIP no-pyyaml" "$WORK/yaml.out"; then
  echo "  ⊘ workflow YAML 結構檢查 — SKIPPED（此環境沒有 PyYAML）"
elif grep -q "^FAIL" "$WORK/yaml.out"; then
  while IFS= read -r line; do bad "workflow" "${line#FAIL }"; done < <(grep "^FAIL" "$WORK/yaml.out")
elif grep -q "^DONE" "$WORK/yaml.out"; then
  ok "workflow YAML 可解析，且 draft→build→gate→finalize／sha256／平台宣稱一致"
else
  bad "workflow YAML 檢查本身失敗" "$(head -3 "$WORK/yaml.out" | tr '\n' ' ')"
fi

# --------------------------------------- 074：get.sh 缺 checksum 必須拒裝
GDIR="$WORK/getrepo"
mkdir -p "$GDIR/bin" "$GDIR/out"
cp scripts/get.sh "$GDIR/"
# 假 curl／gh：主資產抓得到，.sha256 一律 404（中間人只丟掉那一個請求的情境）。
# 主資產是一份「內容完全正常」的 tar.gz（fail-open 版本會一路裝完），
# 只有 .sha256 抓不到 —— 這正是中間人丟掉單一請求的情境。
mkdir -p "$GDIR/payload"
printf '#!/bin/sh\necho "interact-ai 9.9.9"\n' > "$GDIR/payload/interact-ai"
chmod +x "$GDIR/payload/interact-ai"
tar -czf "$GDIR/payload.tar.gz" -C "$GDIR/payload" interact-ai
cat > "$GDIR/bin/curl" <<STUB
#!/usr/bin/env bash
DEST=""; ARGS=("\$@")
for ((i=0; i<\${#ARGS[@]}; i++)); do [[ "\${ARGS[\$i]}" == "-o" ]] && DEST="\${ARGS[\$((i+1))]}"; done
LAST="\${ARGS[\$((\${#ARGS[@]}-1))]}"
case "\$LAST" in
  *releases/latest) echo '{"tag_name": "v9.9.9"}'; exit 0 ;;
  *.sha256) exit 22 ;;
esac
[[ -n "\$DEST" ]] && cp "$GDIR/payload.tar.gz" "\$DEST"
exit 0
STUB
chmod +x "$GDIR/bin/curl"
printf '#!/usr/bin/env bash\nexit 1\n' > "$GDIR/bin/gh"; chmod +x "$GDIR/bin/gh"
( cd "$GDIR" && PATH="$GDIR/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
    /bin/bash get.sh --cli-only --version v9.9.9 --bin-dir "$GDIR/out" ) >"$WORK/get.out" 2>&1
GRC=$?
check "get.sh 抓不到 .sha256 時拒絕安裝（fail-closed）" "$([[ "$GRC" != 0 ]] && echo 0 || echo 1)" \
  "rc=$GRC $(tail -3 "$WORK/get.out" | tr '\n' ' ')"
[[ ! -f "$GDIR/out/interact-ai" ]]
check "get.sh 未驗證時沒有把二進位裝進 bin-dir" $?
grep -q "INTERACT_AI_ALLOW_UNVERIFIED_DOWNLOAD" "$GDIR/get.sh"
check "get.sh 提供明示的逃生門（而不是預設跳過驗證）" $?

# --------------------------------------- release.yml 內嵌 shell／python 必須可執行
python3 scripts/tests/release-yml-embedded.py > "$WORK/embed.out" 2>&1
if grep -q "SKIP no-pyyaml" "$WORK/embed.out"; then
  echo "  ⊘ release.yml 內嵌 shell／python 行為測試 — SKIPPED（此環境沒有 PyYAML）"
elif grep -q "^FAIL" "$WORK/embed.out"; then
  while IFS= read -r line; do bad "release.yml embedded" "${line#FAIL }"; done < <(grep "^FAIL" "$WORK/embed.out")
elif grep -q "^DONE" "$WORK/embed.out"; then
  ok "release.yml 內嵌 shell 可解析，finalize 缺資產會擋、desktop 會算 .sha256"
else
  bad "release.yml 內嵌檢查本身失敗" "$(head -5 "$WORK/embed.out" | tr '\n' ' ')"
fi

# --------------------------- 076：secret 掃描的正則必須認得常見 token 形狀（不只 ghp_）
SECRET_RE="$(grep -oE "git grep -nE -- '[^']+'" scripts/release-verify.sh | head -1 | sed -E "s/^git grep -nE -- '//; s/'$//")"
if [[ -z "$SECRET_RE" ]]; then
  bad "secret 掃描正則可從 release-verify.sh 取出" "找不到 git grep -nE 那行"
else
  for sample in \
    "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123" \
    "gho_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123" \
    "github_pat_11ABCDEFG_abcdefghijklmnopqrstuvwxyz0123456789" \
    "sk-ant-api03-abcdefghijklmnopqrstuvwxyz" \
    "sk-proj-abcdefghijklmnopqrstuvwxyz0123" \
    "AKIAIOSFODNN7EXAMPLE" \
    "AIzaSyA1234567890abcdefghijklmnopqrstuvw" \
    "xoxb-1234567890-abcdefghij" \
    "-----BEGIN OPENSSH PRIVATE KEY-----"; do
    printf '%s\n' "$sample" | grep -qE -- "$SECRET_RE"; check "secret 掃描認得 ${sample:0:14}…" $?
  done
  for benign in "ghost_town_30_characters_long_word" "AKIA short" "sk-1234" "xox-not-a-token"; do
    if printf '%s\n' "$benign" | grep -qE -- "$SECRET_RE"; then bad "secret 掃描不得誤判：$benign"; else ok "secret 掃描不誤判：$benign"; fi
  done
fi

# ------------------------ v0.6.0 Release run 33918252926：Windows 上 CRLF 清單讓 gh 找不到 .sha256
# release.yml 的上傳迴圈必須把每行尾端的 \r 去掉；用假的 gh 重現：檔名帶 \r 就失敗。
UPLOAD_LOOP="$(sed -n '/while IFS= read -r f; do/,/done < sha256-files.txt/p' .github/workflows/release.yml | sed 's/^ *//')"
if [[ -z "$UPLOAD_LOOP" ]]; then
  bad "release.yml 的 .sha256 上傳迴圈可被取出"
else
  mkdir -p "$WORK/crlf" && printf 'a.msi.sha256\r\nb.exe.sha256\r\n' > "$WORK/crlf/sha256-files.txt"
  touch "$WORK/crlf/a.msi.sha256" "$WORK/crlf/b.exe.sha256"
  ( cd "$WORK/crlf" && gh() { local f="$4"; [[ "$f" == *$'\r' ]] && { echo "gh: no matches found for \`$f\`" >&2; return 1; }; [[ -f "$f" ]] || { echo "gh: missing $f" >&2; return 1; }; echo "uploaded $f"; }
    export -f gh 2>/dev/null || true; GITHUB_REF_NAME=v0.0.0 GITHUB_REPOSITORY=x/y bash -c "$(declare -f gh); set -uo pipefail; $UPLOAD_LOOP" ) > "$WORK/crlf.out" 2>&1
  check "release.yml 上傳迴圈對 CRLF 清單（Windows python）仍能上傳每個 .sha256" $? "$(tail -2 "$WORK/crlf.out" | tr '\n' '|')"
  [[ "$(grep -c '^uploaded ' "$WORK/crlf.out")" == "2" ]]; check "CRLF 清單的兩個 .sha256 都被上傳" $?
fi

# --------------------------------------- 078：crate 版本政策沒有白名單
# v0.6.0 時 interaction-adapter-declarative／adapters-media 寫死 0.2.0，release-verify 以 ⚠ 白名單放行；
# v0.6.x 起兩者都 `version.workspace = true`，白名單移除：任何寫死自有版本的 crate 都要讓關卡紅燈。
DDIR="$WORK/verifyrepo-drift"
rm -rf "$DDIR"; cp -R "$VDIR" "$DDIR"
mkdir -p "$DDIR/crates/stray-crate" "$DDIR/adapters/good-adapter"
printf '[package]\nname = "stray-crate"\nversion = "0.2.0"\nedition = "2021"\n' > "$DDIR/crates/stray-crate/Cargo.toml"
printf '[package]\nname = "good-adapter"\nversion.workspace = true\nedition = "2021"\n' > "$DDIR/adapters/good-adapter/Cargo.toml"
( cd "$DDIR" && PATH="$DDIR/bin:$PATH" /bin/bash scripts/release-verify.sh 9.9.9 --skip-ci ) > "$WORK/verify-drift.out" 2>&1
DRC=$?
check "release-verify.sh：寫死自有版本的 crate 讓關卡 exit 非 0（沒有白名單）" "$([[ "$DRC" != 0 ]] && echo 0 || echo 1)" "rc=$DRC"
grep -q "crates/stray-crate/Cargo.toml=0.2.0" "$WORK/verify-drift.out"
check "release-verify.sh 指名漂移的 crate 與它的版本" $? "$(grep -i 'workspace' "$WORK/verify-drift.out" | tr '\n' '|')"
! grep -q "good-adapter" "$WORK/verify-drift.out"
check "release-verify.sh 不把 version.workspace = true 的 crate 列成漂移" $?
! grep -qiE '已知版本漂移|KNOWN' "$WORK/verify-drift.out"
check "release-verify.sh 不再輸出任何『已知版本漂移』白名單字樣" $?
grep -q '^version\.workspace = true' crates/interaction-adapter-declarative/Cargo.toml \
  && grep -q '^version\.workspace = true' adapters/media/Cargo.toml
check "repo 內 interaction-adapter-declarative／adapters-media 都跟著 workspace 版本" $?

echo
echo "release-scripts: ${PASS} passed / ${FAIL} failed"
[[ "$FAIL" == "0" ]]
