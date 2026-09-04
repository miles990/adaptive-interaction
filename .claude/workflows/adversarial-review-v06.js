export const meta = {
  name: 'adversarial-review-v06',
  description: 'v0.6.0 Foundation 對抗審查：preflight → 12 維度（AIP／Session／身分／重連／角色包／UI／證據／發布）find → 獨立懷疑者 verify → findings JSON/Markdown 落盤',
  phases: [
    { title: 'Preflight', detail: '由 git root 解析 repo、檢查規格檔存在、產生 run ID（缺規格即 fail-fast）' },
    { title: 'Find', detail: '各維度並行找缺陷（可帶入 args.seeds 的既有主張）' },
    { title: 'Verify', detail: '每個 finding 由獨立懷疑者反駁；blocker/high 需兩位不同視角皆確認' },
    { title: 'Persist', detail: '寫出 docs/reviews/adversarial/<runId>.{json,md}（不 commit）' },
  ],
}

// 用法（在 repo 內任一目錄的 Claude Code session）：
//   Workflow({ name: 'adversarial-review-v06' })
//   Workflow({ name: 'adversarial-review-v06', args: { seeds: [...], skipDimensions: ['mobile-server'], findModel: 'opus' } })
//   seeds: [{dimension,title,file,line?,severity,claim,evidence}] —— 直接進 Verify（不重找）。
//   skipDimensions: 正在被並行修復、避免對舊碼審查的維度。
//   findModel / verifyModel: 覆寫 finder / verifier 的模型，避開單一模型的速率上限。
//   outDir: 輸出目錄（repo 相對路徑；預設 docs/reviews/adversarial）。
// 可攜性：不硬編任何絕對路徑。Repo 由 `git rev-parse --show-toplevel` 解析；規格必須在
// repo 內（docs/specs/…），缺檔即 throw，不得虛構規格內容。所需 runtime 見 .claude/workflows/README.md。
// 本 workflow 絕不 commit／push／release／deploy；Persist 只寫檔案。

const SPEC_REL = 'docs/aip/README.md'
const DEFAULT_OUT_DIR = 'docs/reviews/adversarial'
const PROTOCOL_REL = 'docs/aip/character-session.md'
const BOUNDARIES_REL = 'docs/aip/architecture-boundaries.md'
const CPP_REL = 'docs/character-protocol/README.md'

const PREFLIGHT_SCHEMA = {
  type: 'object',
  properties: {
    ok: { type: 'boolean' },
    root: { type: 'string', description: 'git rev-parse --show-toplevel 的絕對路徑' },
    head: { type: 'string', description: 'git rev-parse HEAD' },
    headShort: { type: 'string' },
    dirty: { type: 'boolean', description: 'git status --porcelain 非空' },
    utc: { type: 'string', description: 'date -u +%Y%m%dT%H%M%SZ' },
    runId: { type: 'string', description: '<headShort>-<utc>' },
    specSha256: { type: 'string' },
    reason: { type: 'string', description: 'ok=false 時的原因' },
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
    corrected_file_line: { type: 'string', description: '若行號漂移，給目前正確的 file:line' },
    already_fixed: { type: 'boolean', description: '主張在原始碼歷史上成立，但你審查時檔案已被並行修復（confirmed=false 時請標 true 以區分「不是缺陷」與「已修好」）' },
    regression_test_sketch: { type: 'string', description: '若 confirmed：一句話描述能釘死此缺陷的回歸測試' },
  },
  required: ['confirmed', 'reasoning', 'corrected_severity'],
}

const PERSIST_SCHEMA = {
  type: 'object',
  properties: {
    jsonPath: { type: 'string' },
    mdPath: { type: 'string' },
    bytes: { type: 'integer' },
  },
  required: ['jsonPath', 'mdPath'],
}

// ---------------------------------------------------------------------------
// Preflight：由 git root 解析 repo；規格缺失 fail-fast
// ---------------------------------------------------------------------------
phase('Preflight')
const pre = await agent(
  `你是 preflight 檢查員。不要修改任何檔案。在目前工作目錄執行（用 Bash，逐條照抄）：
  ROOT="$(git rev-parse --show-toplevel)" && echo "ROOT=$ROOT"
  cd "$ROOT" && git rev-parse HEAD && git rev-parse --short HEAD && date -u +%Y%m%dT%H%M%SZ
  test -f "${SPEC_REL}" && shasum -a 256 "${SPEC_REL}"
  git status --porcelain | head -1
若 git root 解析失敗、或規格檔 ${SPEC_REL} 不存在，回 ok=false 並在 reason 寫明（不要嘗試從別處找規格、不要憑記憶補內容）。
成功時回 ok=true、root、head、headShort、dirty、utc、specSha256，runId = "<headShort>-<utc>"。`,
  { label: 'preflight', phase: 'Preflight', schema: PREFLIGHT_SCHEMA, effort: 'low' }
)
if (!pre || pre.ok !== true || !pre.root || !pre.runId) {
  throw new Error(
    `adversarial-review-v06 preflight failed: ${pre?.reason || 'no preflight result'} ` +
      `(repo must be a git checkout and the spec must exist at ${SPEC_REL}; nothing was reviewed)`
  )
}
const ROOT = pre.root
const SPEC = `${ROOT}/${SPEC_REL}`
const RUN_ID = pre.runId
const OUT_DIR = typeof args?.outDir === 'string' && args.outDir ? args.outDir : DEFAULT_OUT_DIR
log(`run ${RUN_ID} @ ${ROOT} (HEAD ${pre.headShort}${pre.dirty ? ', dirty worktree' : ''}); spec sha256 ${String(pre.specSha256 || '').slice(0, 12)}`)

const COMMON = `你在 ${ROOT}（Rust workspace＋Tauri 桌面＋iOS 原始碼）。這是 v0.6.0 Foundation 對抗審查（run ${RUN_ID}）。
契約：AIP 1.0 ${SPEC}；Character Session／State Ownership ${ROOT}/${PROTOCOL_REL}；架構邊界 ${ROOT}/${BOUNDARIES_REL}；CPP 1.0（Renderer 契約，不變）${ROOT}/${CPP_REL}；不變量 ${ROOT}/CLAUDE.md。需要時用 sed -n 讀相關章節。
只回報「真實、可驗證」的缺陷：引用實際檔案與行為證據；不確定就不要報。可以跑 cargo test -p <crate>、pnpm vitest run <pattern>、rg、sed；不要啟動 daemon、不要跑 Playwright／pnpm perf（會撞埠與磁碟）、不要 --workspace 全量。
特別留意：誠實階梯（received≠accepted≠applied≠observed≠claimed-completed≠verified）、source 只是宣稱、未知不執行、每個共享狀態只有一個 owner、
revision／sequence 單調、有界集合、無 blocking sleep、production 不濫用 unwrap()、一般模式不外洩 revision／sequence／token／UUID／provider／lease、
fixture／模擬器≠真機、renderer／device／adapter 不能產生 verified、AI 不可授權、estop 重啟不自動恢復、Bonjour 只是發現不是信任。
回報格式走 StructuredOutput。不要修改任何檔案。`

const DIMENSIONS = [
  { key: 'aip-protocol', prompt: `${COMMON}
維度：AIP 協定與版本混淆（契約 §1–§4、§11–§14）。審 crates/interaction-aip/src/**、tests/**、schemas/aip-1.0.schema.json、scripts/aip-codegen.mjs、apps/interaction-desktop/src/aip/**、apps/interaction-ios/InteractionCompanion/Models/AIP*.swift：
protocol version confusion（major/minor 解析邊界、aip/1.10 vs 1.1、空字串、前後空白）、unknown messageType 是否真的不執行（三種語言）、未知選填欄位是否會被 Swift/TS 丟掉、profile 必填是否三語言一致（找不一致案例並用 fixture 證明）、
payload 上限／深度／字串長度是否可被 unicode 或巢狀繞過、canonical_json 對浮點／大整數／重複鍵的穩定性、codegen 是否確定性（跑兩次 diff）、--check 是否真的擋漂移、golden 是否由 Rust 產生。` },
  { key: 'identity-binding', prompt: `${COMMON}
維度：身分綁定與偽造（契約 §5；任務 §11.3、§15）。審 crates/interaction-runtime/src/mobile.rs 的 aip frame 處理、crates/interaction-session/src（submit 管線）、crates/interaction-api/src（/v1/character-session/*）：
偽造 source.id／source.kind、未 auth 連線送 aip、host 是否有「幫忙正規化後執行」的路徑、human-surface 路由是否能假冒 device、agent／session／adapter token 是否摸得到 /v1/character-session、SSE character.session.state 是否只給 human、error payload 是否回顯輸入或洩漏 deviceId／token／路徑。` },
  { key: 'pairing-migration', prompt: `${COMMON}
維度：Bonjour spoofing、endpoint migration hijack、revoked device reuse（任務 §12、§15）。審 crates/interaction-runtime/src/mobile.rs（配對、auth、revoke、conn_id、advertise）、apps/interaction-ios/InteractionCompanion/Services/{ConnectionManager,PairingStore,SessionClient}.swift、crates/interaction-runtime/tests/mobile_loop.rs：
同名 Bonjour 服務或同 hostname 是否會被 App 自動信任（TLS 指紋 pin 是否所有路徑都檢查）、桌面 IP／port 改變時同一身分怎麼遷移、不同身分是否可能沿用舊配對、撤銷後重連是否立即拒絕且 session 成員移除、
replay 舊 pair-response、nonce 重用、配對期燒毀 DoS、rate limit／連線上限／auth timeout 是否對 aip frame 也生效、duplicate connection 的 conn_id 守衛。` },
  { key: 'session-integrity', prompt: `${COMMON}
維度：Session injection、event replay、duplicate side effects、revision rollback、snapshot poisoning（契約 §6–§8；session 文件 §6–§8）。審 crates/interaction-session/src/**、tests/**、crates/interaction-runtime/src/character_session.rs：
跨 session id 注入、messageId 去重環溢出後的重放（sequence／expiresAt 是否真的擋住）、重複 touch 是否造成兩次 intent 或兩次 iphone.touch 觀察、
接收端 revision ≤ local 是否忽略、session-reset 是否可被 device 觸發、snapshot hash 是否被驗、日誌溢出時 resume 是否誠實回 snapshot、tick 是否有界、成員上限、每個 Output envelope 是否 validate、
device 送 task.*／runtime.* 與 verified 是否 scope-denied、emergency 中互動是否拒絕、celebrate 是否在桌面雙播。` },
  { key: 'capability-consent', prompt: `${COMMON}
維度：Capability spoofing、consent double consumption、crash recovery（任務 §15、§14.2）。審 crates/interaction-aip/src/capability.rs、crates/interaction-session、crates/interaction-runtime/src/{character_session,executor,runtime}.rs、crates/interaction-core/src/policy.rs：
renderer 宣告不存在的 intent／input 是否影響其他成員或 host、unsupported 是否能被回報成 observed、帶 consentGrantId 的 command 在離線後是否會自動重送（require-reconfirmation）、maxUses 是否可被 session 路徑繞過、
daemon crash 後 grant／estop／session snapshot 的復活行為（估停不自動恢復、snapshot 損壞→.corrupt＋epoch+1 而非靜默）、Persist 是否在 hot path 做同步 I/O。` },
  { key: 'character-package', prompt: `${COMMON}
維度：Character Package traversal、malicious asset reference、小樞脫核心（架構邊界 §4；CPP §2.1、§9）。審 crates/interaction-character/src/manifest.rs、crates/interaction-character-shu/**、apps/interaction-desktop/src-tauri/src/character_store.rs、apps/interaction-desktop/src/character/{manifest,registry,adapterRegistry}.ts、public/characters/**：
路徑穿越／symlink／magic bytes／大小上限是否每條匯入路徑都檢查、host 注入白名單為空時的 fail-closed、migrator registry 是否可被 manifest 觸發執行、
rg -i 'shu|maid' crates/interaction-character/src 是否真的 0 命中、ref-shape 是否真的沒改核心（git log -p 對照）、CompanionApp／gatewayWiring 是否仍有 entrypoint 分岔、BUNDLED ids 與 index.json 是否一致。` },
  { key: 'renderer-lifecycle', prompt: `${COMMON}
維度：Duplicate subscription、listener／timer／rAF leak、renderer crash propagation（任務 §14、§16.1）。審 apps/interaction-desktop/src/character/{gateway,adapters/*}.ts、src/companion/{CompanionApp.tsx,gatewayWiring.ts}、src/test/adapter-contract.test.ts、apps/interaction-ios/InteractionCompanion/{Services/SessionClient.swift,Views/CharacterView.swift}：
attach／detach 重複是否重複訂閱 SSE／IPC、dispose 後 timer／rAF／listener 是否歸零（用 fake timers 證明或反證）、adapter throw 是否傳染到 gateway／其他 adapter、切換角色時舊 adapter 回執是否污染新 generation、
iOS pending intents／dedupe 是否有界、重連時是否重複 capability／resume 造成雙倍 patch。` },
  { key: 'general-mode-ux', prompt: `${COMMON}
維度：一般模式技術外洩與五入口（任務 §13）。審 apps/interaction-desktop/src/{App.tsx,statusProjection.ts,pages/**,components/**}、src/test/regressions-*.test.tsx、e2e/*.spec.ts：
一般模式是否出現 revision／sequence／epoch／schema／transport／provider／lease／token／UUID／correlation；主入口是否恰好五個；空狀態是否像成功；fixture 是否像真機；claimed-completed 是否像 verified；
「無法恢復，請重新連接」「需要重新確認裝置」等文案是否真的有觸發路徑；390px 五入口可達；鍵盤／focus／aria；Reduced Motion；deep link 相容。` },
  { key: 'evidence-honesty', prompt: `${COMMON}
維度：Fixture／Simulator evidence confusion、claimed-completed celebration、docs 宣稱（任務 §16.5、§18）。審 docs/aip/*.md、docs/releases/v0.6.0-*.md、docs/acceptance-evidence.md、CHANGELOG.md [Unreleased]、apps/interaction-ios/README.md、scripts/v03-cli-e2e.sh、e2e/*.spec.ts、docs/assets/v06-evidence：
每一句「已驗證／閉環／可運作」是否有對應測試與證據等級；fixture／模擬器有無被寫成真機；celebrate 是否只在 verified 觸發（claimed 不慶祝）；數字是否來自實跑；implemented-unverified 是否誠實。` },
  { key: 'reconnect-recovery', prompt: `${COMMON}
維度：斷線重連與過期播放（任務 §8.5、§11.1 15–19）。審 crates/interaction-session、crates/interaction-runtime/src/{character_session,mobile}.rs、apps/interaction-ios/InteractionCompanion/Services/{SessionClient,ConnectionManager}.swift、crates/interaction-runtime/examples/fake_iphone.rs：
過期 touch 在重連後是否可能被播放、intent 是否 drop-if-offline、resume 缺漏（sequence gap）是否偵測、三連敗是否誠實顯示、presence 逾時與 leave 的區別、桌面視窗 re-hello 是否重複 join、superseded 連線的 Goodbye 是否有測試。` },
  { key: 'runtime-boundaries', prompt: `${COMMON}
維度：Core 不依賴 transport／小樞／iPhone 字面值（任務 §6.1、§21）。審 crates/interaction-session/Cargo.toml、crates/interaction-aip/Cargo.toml、tests/e2e/tests/dependency_boundaries.rs、crates/interaction-runtime/src/{character,activity,sensors,providers}.rs：
純 crate 是否真的無 tokio／I/O、runtime 核心對 "iphone." 字面值的殘留（character.rs is_presentation_surface_actuator、activity.rs、sensors.rs、providers.rs）、Backend／Transport 的 switch-case 是否新增了更多分岔、EventType 新增是否同步 as_str 與 SSE 過濾。` },
  { key: 'release-provenance', prompt: `${COMMON}
維度：Release provenance（任務 §20）。審 scripts/release-{prepare,verify,tag}.sh、scripts/release.sh、.github/workflows/{ci,release}.yml、Cargo.toml／package.json／tauri.conf.json 版本、CHANGELOG 結構、apps/interaction-desktop/src-tauri/src/lib.rs 的 CHANGELOG claim-check 測試：
verify 關卡能否被繞過（--skip-ci 濫用、secret 掃描漏網、版本不同步）、tag 是否可能指向未跑 CI 的 commit、release.yml 的 checksum／SBOM／provenance 能力（若無 SBOM 要誠實標）、prepare 對空 Unreleased 的處理、CI 是否包含 aip:check 與 dependency_boundaries。` },
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
  { id: 'reproducibility', text: '視角＝可重現性：親自打開檔案、必要時寫最小腳本或跑既有測試證明主張是否成立；若是「缺少功能」主張，grep 全 repo 確認確實不存在。' },
  { id: 'spec-honesty', text: '視角＝規格與誠實階梯：對照規格原文與 CLAUDE.md 不變量，判斷這是否真的違反規格/不變量，還是只是風格偏好或已被其他機制覆蓋。' },
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
親自驗證。只有證據確鑿才 confirmed=true；重複/風格/主觀偏好/已被測試覆蓋的防護/已被修復（檔案可能在你審查時已更新，請以目前檔案內容為準並回報行號漂移）一律 refute。
不確定時 confirmed=false。`,
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
// 1) seeds 直接驗證（不重找）
const seedTask = parallel(seeds.map((s) => () => verifyOne({ ...s, seed: true }, s.dimension || 'seed')))

// 2) 各維度找新缺陷 → 去重 → 驗證（pipeline：找到就驗）
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
  dirtyWorktree: Boolean(pre.dirty),
  generatedAtUtc: pre.utc,
  spec: { path: SPEC_REL, sha256: pre.specSha256 || '' },
  dimensions: dims.map((d) => d.key),
  totals: { reviewed: all.length, confirmed: confirmed.length, fixedMeanwhile: fixedMeanwhile.length, refuted: refuted.length, unverified: unverified.length, seeds: seeds.length },
  confirmed: confirmed.map(record),
  fixedMeanwhile: fixedMeanwhile.map(record),
  refuted: refuted.map(record),
  unverified: unverified.map(record),
}

// ---------------------------------------------------------------------------
// Persist：findings JSON＋Markdown 落盤（不 commit）
// ---------------------------------------------------------------------------
phase('Persist')
const persisted = await agent(
  `你是報告書記。把下面這份 JSON 一字不改寫到 ${ROOT}/${OUT_DIR}/${RUN_ID}.json（先 mkdir -p 目錄；用 Write 工具寫入完整內容，不得截斷、不得改寫欄位），
再依它產生一份 ${ROOT}/${OUT_DIR}/${RUN_ID}.md（繁體中文）：
標題含 run ID、HEAD、worktree 是否 dirty、規格路徑＋sha256、各維度；
一張總表（reviewed/confirmed/fixed-meanwhile/refuted/unverified）；
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
