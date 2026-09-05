export const meta = {
  name: 'adversarial-review-v06x',
  description: 'v0.6.x 可維護性與一般模式收斂分支的對抗審查：12 維度（含誠實階梯／證據對帳）只審本分支相對 main 的 diff，每個 finding 由獨立懷疑者反駁，blocker/high 雙視角',
  phases: [
    { title: 'Preflight', detail: '由 git root 解析 repo、確認 base commit 與進度文件存在、產生 run ID' },
    { title: 'Find', detail: '各維度並行找缺陷（只看 base..HEAD 的變更與其呼叫者）' },
    { title: 'Verify', detail: '每個 finding 由獨立懷疑者反駁；blocker/high 需兩位不同視角皆確認' },
    { title: 'Persist', detail: '寫出 docs/reviews/adversarial/<runId>.{json,md}（不 commit）' },
  ],
}

// 用法（在 repo 內任一目錄的 Claude Code session）：
//   Workflow({ name: 'adversarial-review-v06x' })
//   Workflow({ name: 'adversarial-review-v06x', args: { base: '8f52837', seeds: [...], skipDimensions: [...], findModel: 'opus', verifyModel: 'sonnet' } })
//   base: 分支起點 commit（預設 8f52837 ＝ origin/main 的 v0.6.0 發布後 HEAD）。審查範圍＝ `git diff <base>..HEAD`。
//   seeds: [{dimension,title,file,line?,severity,claim,evidence}] —— 直接進 Verify（不重找）。
//   skipDimensions / findModel / verifyModel / outDir：同 adversarial-review-v06.js。
// 可攜性：不硬編絕對路徑；preflight 以 git rev-parse 解析。本 workflow 絕不 commit／push／release／deploy。

const DEFAULT_OUT_DIR = 'docs/reviews/adversarial'
const DEFAULT_BASE = '8f52837'
const PROGRESS_REL = 'docs/releases/v0.6.x-maintainability-progress.md'
const AIP_REL = 'docs/aip/README.md'
const SESSION_REL = 'docs/aip/character-session.md'
const DEVICE_REL = 'docs/aip/device-profile.md'
const UX_REL = 'docs/aip/general-mode-ux.md'

const PREFLIGHT_SCHEMA = {
  type: 'object',
  properties: {
    ok: { type: 'boolean' },
    root: { type: 'string' },
    head: { type: 'string' },
    headShort: { type: 'string' },
    base: { type: 'string', description: 'git rev-parse <base>' },
    dirty: { type: 'boolean' },
    utc: { type: 'string' },
    runId: { type: 'string', description: '<headShort>-<utc>' },
    changedFiles: { type: 'integer', description: 'git diff --name-only base..HEAD | wc -l' },
    reason: { type: 'string' },
  },
  required: ['ok'],
}

const FINDINGS_SCHEMA = {
  type: 'object',
  properties: {
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          title: { type: 'string' },
          file: { type: 'string' },
          line: { type: 'integer' },
          severity: { type: 'string', enum: ['critical', 'blocker', 'high', 'medium', 'low'] },
          claim: { type: 'string', description: '缺陷主張：具體、可反駁' },
          evidence: { type: 'string', description: '程式碼/測試證據（引用實際行）' },
          fix_sketch: { type: 'string' },
        },
        required: ['title', 'file', 'severity', 'claim', 'evidence'],
      },
    },
  },
  required: ['findings'],
}

const VERDICT_SCHEMA = {
  type: 'object',
  properties: {
    confirmed: { type: 'boolean' },
    reasoning: { type: 'string' },
    corrected_severity: { type: 'string', enum: ['blocker', 'high', 'medium', 'low', 'not-a-bug'] },
    corrected_file_line: { type: 'string' },
    already_fixed: { type: 'boolean' },
    regression_test_sketch: { type: 'string' },
  },
  required: ['confirmed', 'reasoning', 'corrected_severity'],
}

const PERSIST_SCHEMA = {
  type: 'object',
  properties: { jsonPath: { type: 'string' }, mdPath: { type: 'string' }, bytes: { type: 'integer' } },
  required: ['jsonPath', 'mdPath'],
}

const BASE_ARG = typeof args?.base === 'string' && args.base ? args.base : DEFAULT_BASE

phase('Preflight')
const pre = await agent(
  `你是 preflight 檢查員。不要修改任何檔案。在目前工作目錄執行（用 Bash，逐條照抄）：
  ROOT="$(git rev-parse --show-toplevel)" && echo "ROOT=$ROOT"
  cd "$ROOT" && git rev-parse HEAD && git rev-parse --short HEAD && git rev-parse ${BASE_ARG} && date -u +%Y%m%dT%H%M%SZ
  test -f "${PROGRESS_REL}" && test -f "${AIP_REL}" && test -f "${SESSION_REL}" && echo docs-ok
  git diff --name-only ${BASE_ARG}..HEAD | wc -l
  git status --porcelain | head -1
若 git root 解析失敗、base commit 不存在、或進度／契約文件不存在，回 ok=false 並在 reason 寫明。
成功時回 ok=true、root、head、headShort、base（完整 hash）、dirty、utc、changedFiles，runId = "<headShort>-<utc>"。`,
  { label: 'preflight', phase: 'Preflight', schema: PREFLIGHT_SCHEMA, effort: 'low' }
)
if (!pre || pre.ok !== true || !pre.root || !pre.runId) {
  throw new Error(`adversarial-review-v06x preflight failed: ${pre?.reason || 'no preflight result'} (nothing was reviewed)`)
}
const ROOT = pre.root
const RUN_ID = pre.runId
const BASE = pre.base || BASE_ARG
const OUT_DIR = typeof args?.outDir === 'string' && args.outDir ? args.outDir : DEFAULT_OUT_DIR
log(`run ${RUN_ID} @ ${ROOT} (HEAD ${pre.headShort}${pre.dirty ? ', dirty worktree' : ''}); base ${String(BASE).slice(0, 12)}; ${pre.changedFiles ?? '?'} changed files`)

const COMMON = `你在 ${ROOT}（Rust workspace＋Tauri 桌面＋iOS 原始碼）。這是 v0.6.x 可維護性與一般模式收斂分支的對抗審查（run ${RUN_ID}）。
審查範圍＝本分支相對 base 的變更：先跑 git diff --stat ${BASE}..HEAD 與 git diff ${BASE}..HEAD -- <你的維度檔案>，再讀變更處的呼叫者。舊碼未動的部分只在「本分支的變更讓它變錯」時才報。
契約：AIP 1.0 ${ROOT}/${AIP_REL}；Character Session ${ROOT}/${SESSION_REL}；Device Profile ${ROOT}/${DEVICE_REL}；一般模式 ${ROOT}/${UX_REL}；不變量 ${ROOT}/CLAUDE.md；
本分支的需求矩陣與已落地清單 ${ROOT}/${PROGRESS_REL}；CHANGELOG [Unreleased] 段是本分支的自我宣稱（對帳用）。需要時用 sed -n 讀相關章節。
只回報「真實、可驗證」的缺陷：引用實際檔案與行為證據；不確定就不要報。可以跑 cargo test -p <crate> --test <file>、pnpm vitest run <pattern>、bash scripts/tests/*.sh、rg、sed；
不要啟動 daemon、不要跑 Playwright／pnpm perf／cargo test --workspace（會撞埠、磁碟與其他 agent）。**絕對不要** git commit／stash／checkout／reset／restore／clean，也不要修改任何檔案。
特別留意：誠實階梯（queued≠completed、acknowledged≠completed、completed≠verified、模擬器≠真機）、有效值＝min(AI、使用者、session、裝置上限、預算)、AI 不可授權／解除 estop、
每個共享狀態只有一個 owner、revision／sequence 單調、有界集合、無 blocking sleep、production 不濫用 unwrap()、一般模式不外洩 revision／sequence／token／UUID／provider id／lease、
安全訊息永遠落 system.text＋可信 overlay、核心不得認得特定裝置／角色、預設關閉的東西重啟不自動恢復。回報格式走 StructuredOutput。`

const DIMENSIONS = [
  { key: 'session-client-rollback', prompt: `${COMMON}
維度：桌面 Session client 的 rollback 防護（M1 §2.1）。審 apps/interaction-desktop/src/aip/sessionClient.ts、src/aip/canonical.ts、src/hooks 或 pages 內使用 reducer 的接線（rg -n "sessionClient|applyIncoming|connectionKey" src）、src/test/sessionClient*.test.ts：
嚴格 revision／epoch 比較是否有被繞過的路徑（缺 revision 變 0、慢 GET 覆蓋新 SSE、resume 與 GET 的競態、daemon 重啟後 sequence 去重）、request generation 是否真的作廢舊回應、reset 契約（只有 session-reset＋epoch 不同才清）是否與 Rust 一致、hash 核對失敗時的行為是否誠實（不靜默接受）。` },
  { key: 'snapshot-format-persist', prompt: `${COMMON}
維度：快照格式版本、遷移與持久化順序（M1 §2.2／§2.3）。審 crates/interaction-session/src/{snapshot,session,ports}.rs、crates/interaction-runtime/src/character_session.rs（store／restore／persist）、tests/fixtures/character-session/**、crates/interaction-runtime/tests/character_session_loop.rs：
五種載入結果是否互斥完整、未來版本是否真的不被覆寫（parked 之後所有寫入路徑都擋住了嗎，含 emergency／reset）、備份檔命名衝突、遷移中斷的原子性、(epoch, revision) guard 是否與 save 在同一把鎖內、SaveOutcome 被呼叫端忽略的地方、diagnostics store 物件是否能誤導（parked 但 UI 說正常）、format 是否真的不進 hash。` },
  { key: 'hash-numeric-contract', prompt: `${COMMON}
維度：hash／數字契約與三端 fixtures（M1 §2.4）。審 crates/interaction-aip/tests/fixtures/state-hash-*.json、manifest.json、crates/interaction-session/tests/state_hash_fixtures.rs、apps/interaction-desktop/src/aip/canonical.ts、apps/interaction-ios/InteractionCompanion/Models/SemanticJSON*.swift 與 AIPConformanceTests、scripts/aip-codegen.mjs：
stateHashDoublePaths 由 schemars 推導的假設是否會在新增 f64 欄位時靜默失效、-0／NaN／大整數／unicode 正規化在三端是否一致（找反例並用 fixture 證明）、AIP_UPDATE_FIXTURES=1 產生器是否確定性、drift gate 是否真的擋得住（MAX_PROJECTED_UNSUPPORTED_INPUTS 與 AIP 的 32 不同步時誰會叫）。` },
  { key: 'sensor-source-stop', prompt: `${COMMON}
維度：SensorSource port 與統一停止協調器（M2 §3.1，X1／X2／S4）。審 crates/interaction-runtime/src/{sensor_source,sensors,providers}.rs、mobile.rs 的 request_stop／release、declarative_session.rs 的 DeclarativeSensorSource、tests/{sensors_loop,providers_loop,declarative_session_loop}.rs、interaction-api routes 的 stop／emergency／receptor delete：
emergency_stop 與「停止所有感測」是否真的走同一份 sweep、unreported 高風險受器是否可能被漏算（宣告但未 register 來源、來源 release 後受器仍啟用）、有界登記表滿了之後的行為是否誠實、request_stop 先關旗標再送 stop-all 的視窗、Unknown／Unreachable／Refused 的投影是否會被上層變成「已停止」、provider 停用後高風險能力重啟是否會自動恢復。` },
  { key: 'capability-registry-owner', prompt: `${COMMON}
維度：能力宣告 registry 的所有權與 provider 生命週期（M2 §3.2、v0.5.1 #4）。審 crates/interaction-runtime/src/{runtime,providers,character}.rs 中 ProviderCapabilityRegistry／CapabilityDeclarationsView／retract_provider_capabilities／transition_provider、interaction-api 的 GET /v1/providers/declarations：
是否仍有第二個寫入者、唯讀 view 是否能被轉型成可寫、transition 到 Disabled／Revoked 後受器旗標是否全部關閉（含 mobile 與宣告式）、回 Available 是否有任何能力被自動打開、declarations 端點是否洩漏 token／路徑／secret://。` },
  { key: 'declarative-aip-binding', prompt: `${COMMON}
維度：宣告式裝置線 v1.1 與第二裝置 AIP profile（M2 §3.3，D1／D2）。審 crates/interaction-adapter-declarative/src/{protocol,lib,link_caps,serial}.rs、tests/{aip_link,esp32_sim_conformance}.rs、crates/interaction-runtime/src/{declarative_session,character_session}.rs（裝置出站登記表、transport 標籤、identityStrength）、tests/declarative_session_loop.rs、scripts/esp32-serial-sim.py、firmware/esp32-companion/：
未配對／舊世代連線的 aip 是否有任何放行路徑、Party::device 綁定是否可被 spec 的 deviceId 與 hello.deviceId 不一致繞過、8 KiB／639 bytes／16 KiB 三層上限是否可被繞過或造成無界緩衝、出站登記表是否無界或在 retire 後殘留、廣播是否會送到已撤銷的裝置、稽核 transport／identityStrength 是否由 Runtime 決定而非裝置自報、核心是否出現 iphone／serial 字面分岔、模擬器結果是否在任何文件被寫成真板、declarative_session_loop 在預設並行下是否仍 flaky（實跑 3 次）。` },
  { key: 'character-settings-binding', prompt: `${COMMON}
維度：角色專屬設定綁定 adapter／package（M2 §3.4、v0.6.0 #17）與模組拆分（§3.5）。審 apps/interaction-desktop/src/companion/settingsTransfer.ts、src/character/adapterRegistry.ts、adapters/*/meta、src/pages/companion/**、src/statusProjection/**、src/test/companion-gateway-wiring.test.ts：
舊小樞家族的寬容是否能被非小樞 id 冒用、declared-but-invalid 是否仍拒絕、匯入是否能夾帶另一個角色的配色成為死值、companionScene 全域白名單的殘留、頁面層是否又出現 entrypoint／pack id 字面（棘輪）、拆分後是否有兩份同名邏輯。` },
  { key: 'general-mode-ux', prompt: `${COMMON}
維度：一般模式首屏、同步卡 action id、移除終態、任務驗收（M3 §4.1–§4.4）。審 apps/interaction-desktop/src/pages/CompanionPage.tsx、src/pages/companion/**、src/companion/presets.ts、src/statusProjection/characterSync.ts、src/routing.ts、src/useNavigation.ts、src/App.tsx、src/components/GlobalSearch.tsx、e2e/general-mode-tasks.spec.ts、docs/releases/v0.6.x-general-mode-tasks.md：
12 個同步狀態是否有無 action 或 action 落點錯的組合、local-only 是否掩蓋安全上需要 needs-reconfirmation 的情況、deep link 帶 options 的殘留（下一次導覽是否沾到舊 options）、首屏是否外洩 revision／sequence／provider id／UUID、陪伴預設兩段寫入的非原子視窗、⌘K 面板焦點陷阱是否影響其他 modal、390px 的溢出（用 vitest／jsdom 能測的部分）、任務量測數字是否手填。` },
  { key: 'ios-lifecycle-heartbeat', prompt: `${COMMON}
維度：iOS 生命週期與 heartbeat（M2／M3 iOS 部分）。審 apps/interaction-ios/InteractionCompanion/Services/{AppLifecycle,SessionClient}.swift 與對應 XCTest、docs/aip/iphone-companion.md：
背景／前景切換時 presence 是否可能說謊（背景仍 online）、15 s heartbeat 在 App 被暫停時的行為、heartbeat 回應被忽略的路徑、resume 與 lifecycle 的競態、任何「模擬器通過」被寫成真機的地方。只審 Swift 原始碼與文件，不需要 Xcode。` },
  { key: 'docs-evidence-release', prompt: `${COMMON}
維度：文件對帳、證據索引、release 腳本（M4 §5.1–§5.3）。審 docs/releases/evidence-index.json、scripts/tests/{docs-claims,release-scripts}.sh、scripts/release-{prepare,verify,tag}.sh、apps/interaction-desktop/src-tauri/src/lib.rs 的 changelog_click_through_claims、CHANGELOG.md [Unreleased]、docs/releases/v0.6.x-*.md、docs/aip/*.md 本分支改過的段落：
CHANGELOG 每條宣稱是否都有對應 commit 與測試（抽 5 條實際核對）、evidence-index 的 tag→commit 是否與 git 一致、docs-claims lint 是否能被繞過（改字就過）、release-verify 的退出碼在各失敗路徑是否非零、crate 版本政策是否一致（rg -n '^version' crates/*/Cargo.toml adapters/*/Cargo.toml）、文件中「已驗收／已驗證」是否有模擬器冒充真機、以及任何過期的「唯一實作／尚未落地」句子。` },
  { key: 'honesty-ladder-audit', prompt: `${COMMON}
維度：誠實階梯稽核（跨模組）。從 git diff ${BASE}..HEAD 中找出所有新增或改寫的「成功／已停止／已同步／verified／completed／已確認」文字與布林（Rust、TS、Swift、文件），逐一追到它的證據來源：
是否有 queued／sent／requested 被投影成 completed、acknowledged 被當 completed、completed 被當 verified、fixture／模擬器結果被寫成真機、unknown 被四捨五入成 stopped、測試數字在文件與實際輸出不一致（抽查 CHANGELOG／progress／final-report 的數字對 git log 與測試檔）。每個 finding 必須指出「宣稱在哪、證據在哪、差距是什麼」。` },
  { key: 'async-bounds-unwrap', prompt: `${COMMON}
維度：本分支新增 Rust／TS 程式碼的非同步衛生與有界性。對 git diff ${BASE}..HEAD 的每個 .rs／.ts 新增區塊：
無界 Vec／Map／channel／queue、blocking sleep 或 std::sync::Mutex 跨 await、production 路徑的 unwrap()／expect()／panic、沒有 timeout 的 await、spawn 出去沒有 abort／JoinHandle 的 task、TaskGroup drop 是否真的 abort、retry 退避無上限、log 或稽核在 hot path 無界成長、TS 端 setTimeout／interval 未清理、React effect 缺 cleanup、Promise 未 catch。用 rg 找候選再親自讀上下文；只報有實際觸發條件的。` },
]

const seeds = Array.isArray(args?.seeds) ? args.seeds : []
const findOpts = args?.findModel ? { model: args.findModel } : {}
const verifyOpts = args?.verifyModel ? { model: args.verifyModel } : {}
const skip = new Set(Array.isArray(args?.skipDimensions) ? args.skipDimensions : [])
const dims = DIMENSIONS.filter((d) => !skip.has(d.key))
log(`dimensions: ${dims.length} (skipped: ${[...skip].join(',') || 'none'}); seeds: ${seeds.length}`)

const keyOf = (f) => `${String(f.file || '').split(/[\s|(]/)[0].split('/').pop()}::${String(f.title || '').toLowerCase().slice(0, 28)}`
const seen = new Set(seeds.map(keyOf))
let findingSeq = 0
const nextFindingId = (dimKey) => `F-${RUN_ID}-${dimKey}-${String(++findingSeq).padStart(3, '0')}`

const LENSES = [
  { id: 'reproducibility', text: '視角＝可重現性：親自打開檔案、必要時寫最小腳本或跑既有測試證明主張是否成立；若是「缺少功能」主張，rg 全 repo 確認確實不存在。' },
  { id: 'spec-honesty', text: '視角＝規格與誠實階梯：對照契約原文與 CLAUDE.md 不變量，判斷這是否真的違反規格／不變量，還是只是風格偏好或已被其他機制覆蓋。' },
]

const verifyOne = (f, dimKey) => {
  const id = f.id || nextFindingId(dimKey)
  const isHot = ['blocker', 'critical', 'high'].includes(String(f.severity))
  const lenses = isHot ? LENSES : [LENSES[0]]
  return parallel(
    lenses.map((lens, i) => () =>
      agent(
        `${COMMON}
你是獨立懷疑者（${lens.text}）。用力嘗試【反駁】以下 finding（維度 ${dimKey}，id ${id}）：
title: ${f.title}
file: ${f.file}${f.line ? ':' + f.line : ''}
severity: ${f.severity}
claim: ${f.claim}
evidence: ${f.evidence}
親自驗證。只有證據確鑿才 confirmed=true；重複／風格／主觀偏好／已被測試覆蓋的防護／已被修復（以目前檔案內容為準並回報行號漂移）一律 refute。不確定時 confirmed=false。`,
        { label: `verify${i + 1}:${dimKey}:${String(f.title || '').slice(0, 22)}`, phase: 'Verify', schema: VERDICT_SCHEMA, ...verifyOpts }
      ).then((v) => (v ? { lens: lens.id, ...v } : null))
    )
  ).then((vs) => {
    const votes = vs.filter(Boolean)
    const confirmed = votes.length > 0 && votes.every((v) => v.confirmed === true && v.corrected_severity !== 'not-a-bug')
    const fixedMeanwhile = !confirmed && votes.length > 0 && votes.some((v) => v.already_fixed === true)
    return { ...f, id, dimension: dimKey, verdicts: votes, confirmed, fixedMeanwhile, seed: Boolean(f.seed) }
  })
}

phase('Find')
const seedTask = parallel(seeds.map((s) => () => verifyOne({ ...s, seed: true }, s.dimension || 'seed')))
const foundTask = pipeline(
  dims,
  (d) => agent(d.prompt, { label: `find:${d.key}`, phase: 'Find', schema: FINDINGS_SCHEMA, ...findOpts }),
  (result, d) => {
    const all = result?.findings ?? []
    const capped = all.slice(0, 10)
    if (all.length > capped.length) log(`find:${d.key}: ${all.length - capped.length} findings beyond the per-dimension cap of 10 were dropped`)
    const fresh = capped.filter((f) => {
      const k = keyOf(f)
      if (seen.has(k)) return false
      seen.add(k)
      return true
    })
    log(`find:${d.key} → ${all.length} findings, ${fresh.length} new after dedupe`)
    return parallel(fresh.map((f) => () => verifyOne(f, d.key)))
  }
)

const [seedResults, foundResults] = await Promise.all([seedTask, foundTask])
const all = [...seedResults.filter(Boolean), ...foundResults.flat().filter(Boolean)]
const confirmed = all.filter((f) => f.confirmed)
const fixedMeanwhile = all.filter((f) => !f.confirmed && f.fixedMeanwhile)
const refuted = all.filter((f) => !f.confirmed && !f.fixedMeanwhile)
const unverified = all.filter((f) => !f.verdicts || f.verdicts.length === 0)
const sevOf = (f) => (f.verdicts?.[0]?.corrected_severity) || f.severity
const order = { blocker: 0, critical: 0, high: 1, medium: 2, low: 3 }
confirmed.sort((a, b) => (order[sevOf(a)] ?? 9) - (order[sevOf(b)] ?? 9))
log(`confirmed: ${confirmed.length} (seeds ${confirmed.filter((f) => f.seed).length}), fixed-meanwhile: ${fixedMeanwhile.length}, refuted: ${refuted.length}, unverified(agent failures): ${unverified.length}, total reviewed: ${all.length}`)

const record = (f) => ({
  id: f.id,
  dimension: f.dimension,
  seed: f.seed,
  severity: sevOf(f),
  reportedSeverity: f.severity,
  title: f.title,
  file: f.verdicts?.find((v) => v.corrected_file_line)?.corrected_file_line || `${f.file}${f.line ? ':' + f.line : ''}`,
  claim: f.claim,
  evidence: f.evidence,
  verdict: f.confirmed ? 'confirmed' : f.fixedMeanwhile ? 'fixed-meanwhile' : (f.verdicts?.length ? 'refuted' : 'unverified'),
  verdicts: (f.verdicts || []).map((v) => ({ lens: v.lens, confirmed: v.confirmed, severity: v.corrected_severity, reasoning: v.reasoning, alreadyFixed: Boolean(v.already_fixed) })),
  fix: f.fix_sketch || '',
  regressionTest: (f.verdicts || []).map((v) => v.regression_test_sketch).filter(Boolean).join(' / '),
})

const report = {
  runId: RUN_ID,
  repo: ROOT,
  head: pre.head,
  base: BASE,
  dirtyWorktree: Boolean(pre.dirty),
  generatedAtUtc: pre.utc,
  scope: `git diff ${BASE}..${pre.head}`,
  dimensions: dims.map((d) => d.key),
  totals: { reviewed: all.length, confirmed: confirmed.length, fixedMeanwhile: fixedMeanwhile.length, refuted: refuted.length, unverified: unverified.length, seeds: seeds.length },
  confirmed: confirmed.map(record),
  fixedMeanwhile: fixedMeanwhile.map(record),
  refuted: refuted.map(record),
  unverified: unverified.map(record),
}

phase('Persist')
const persisted = await agent(
  `你是報告書記。把下面這份 JSON 一字不改寫到 ${ROOT}/${OUT_DIR}/${RUN_ID}.json（先 mkdir -p 目錄；用 Write 工具寫入完整內容，不得截斷、不得改寫欄位），
再依它產生一份 ${ROOT}/${OUT_DIR}/${RUN_ID}.md（繁體中文）：
標題含 run ID、HEAD、base、審查範圍、worktree 是否 dirty、各維度；一張總表（reviewed/confirmed/fixed-meanwhile/refuted/unverified）；
然後 confirmed 依 severity 分節，每項列出 finding ID、severity、file:line、claim、evidence、每位 verifier 的視角與 verdict、fix、regression test；
之後 fixed-meanwhile、refuted、unverified 各一張精簡表（ID、title、file、why）。
絕不執行 git add／commit／push。完成後回傳兩個路徑與 JSON 位元組數。
JSON：
${JSON.stringify(report)}`,
  { label: 'persist', phase: 'Persist', schema: PERSIST_SCHEMA, effort: 'low' }
)
if (!persisted) log('persist agent failed; findings are only in this return value')
else log(`persisted ${persisted.jsonPath} and ${persisted.mdPath}`)

return { ...report, output: persisted }
