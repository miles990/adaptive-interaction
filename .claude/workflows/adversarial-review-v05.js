export const meta = {
  name: 'adversarial-review-v05',
  description: 'v0.5 對抗審查：preflight（git root＋repo 內規格）→ 12 維度 find → 獨立懷疑者 verify → findings JSON/Markdown 落盤',
  phases: [
    { title: 'Preflight', detail: '由 git root 解析 repo、檢查規格檔存在、產生 run ID（缺規格即 fail-fast）' },
    { title: 'Find', detail: '各維度並行找缺陷（可帶入 args.seeds 的既有主張）' },
    { title: 'Verify', detail: '每個 finding 由獨立懷疑者反駁；blocker/high 需兩位不同視角皆確認' },
    { title: 'Persist', detail: '寫出 docs/reviews/adversarial/<runId>.{json,md}（不 commit）' },
  ],
}

// 用法（在 repo 內任一目錄的 Claude Code session）：
//   Workflow({ name: 'adversarial-review-v05' })
//   Workflow({ name: 'adversarial-review-v05', args: { seeds: [...], skipDimensions: ['mobile-server'], findModel: 'opus' } })
//   seeds: [{dimension,title,file,line?,severity,claim,evidence}] —— 直接進 Verify（不重找）。
//   skipDimensions: 正在被並行修復、避免對舊碼審查的維度。
//   findModel / verifyModel: 覆寫 finder / verifier 的模型，避開單一模型的速率上限。
//   outDir: 輸出目錄（repo 相對路徑；預設 docs/reviews/adversarial）。
// 可攜性：不硬編任何絕對路徑。Repo 由 `git rev-parse --show-toplevel` 解析；規格必須在
// repo 內（docs/specs/…），缺檔即 throw，不得虛構規格內容。所需 runtime 見 .claude/workflows/README.md。
// 本 workflow 絕不 commit／push／release／deploy；Persist 只寫檔案。

const SPEC_REL = 'docs/specs/adaptive-interaction-v05-core-experience-prompt.md'
const DEFAULT_OUT_DIR = 'docs/reviews/adversarial'
const PROTOCOL_REL = 'docs/character-protocol/README.md'

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
    `adversarial-review-v05 preflight failed: ${pre?.reason || 'no preflight result'} ` +
      `(repo must be a git checkout and the spec must exist at ${SPEC_REL}; nothing was reviewed)`
  )
}
const ROOT = pre.root
const SPEC = `${ROOT}/${SPEC_REL}`
const RUN_ID = pre.runId
const OUT_DIR = typeof args?.outDir === 'string' && args.outDir ? args.outDir : DEFAULT_OUT_DIR
log(`run ${RUN_ID} @ ${ROOT} (HEAD ${pre.headShort}${pre.dirty ? ', dirty worktree' : ''}); spec sha256 ${String(pre.specSha256 || '').slice(0, 12)}`)

const COMMON = `你在 ${ROOT}（Rust workspace＋Tauri 桌面＋ESP32 韌體＋iOS 原始碼）。這是 v0.5 對抗審查（run ${RUN_ID}）。
規格全文：${SPEC}（需要時用 sed -n 讀相關章節；不要只看標題）。角色呈現協定文件（若存在）：${ROOT}/${PROTOCOL_REL}。
只回報「真實、可驗證」的缺陷：引用實際檔案與行為證據；不確定就不要報。可以跑 cargo test -p <crate>、pnpm test、grep、sed；不要啟動 daemon、不要跑 e2e/playwright（會撞埠）。
特別留意專案不變量：誠實階梯（queued≠completed≠verified、claim 不冒充 verified、unknown 不演成功）、
實體效果絕不自動重送、感測不靜默、AI 不可自我授權、bounded queues、無 blocking sleep 於 async、
production 不濫用 unwrap()、每項設定只有一個主人、一般 UI 不暴露治理術語、模擬器≠真機、
角色呈現層無權限主權（adapter／pack 不能決定 truthState、不能偽造 verified、不能解除 estop）。
回報格式走 StructuredOutput。不要修改任何檔案。`

const DIMENSIONS = [
  {
    key: 'companion-gameplay',
    prompt: `${COMMON}
維度：遊玩層（規格 §3.1、§5、§15.2）。審 apps/interaction-desktop/src/companion/{playfield.ts,rig/stage.ts,director.ts,CompanionApp.tsx,machine.ts} 與 src/test/{playfield,companion,behavior}.test.ts：
truth-state 搶佔是否無漏洞（emergency/offline/paused 凍結一切遊玩？claimed 會不會被遊玩表情蓋掉？）、
Reduced Motion/quiet 覆蓋、物理邊界（NaN/極端 dt/視窗縮放）、hit-rect 與 pointer 競態、
worldBusy 與 Director 互鎖、記憶體/interval 洩漏、每幀配置過多、
§5.1 十二條與 §5.2 十條逐一核對（有無、是否只是資料結構而無行為）、放下四種落地、第 6 種玩具、
多角色互相注意是否真的互動（非單向）、每個高頻反應 3–6 變體與冷卻、各項可分別關閉。`,
  },
  {
    key: 'rig-renderer',
    prompt: `${COMMON}
維度：角色渲染與四段式（規格 §4、§6.2–6.4、§7）。審 apps/interaction-desktop/src/companion/rig/{params.ts,draw.ts,expressions.ts,timeline.ts,renderer.ts,stage.ts}：
clampParams 是否漏欄位、lerp 字串切換的視覺跳變、success claimed/verified 解析是否可被繞過
（alias 或 fallback 鏈讓未驗證顯示綠勾）、truthState 標記完整性、
blink/loop 時間軸在長時間執行（數天）後的數值行為、reduced motion 是否真的靜態、
四段式：exit 段是否真的被 timeline 播放？36 表情各自有幾段？（用 node 腳本統計並引用）、
組合式通道：能否「趴著＋核心顯示 Agent 工作中」（stage.ts 是否整體覆蓋）、
§4.2 服裝表現表 8 部位（頭飾連線狀態、奔跑後歪掉扶正）是否真讀狀態、§4.3 個性是否影響行為。`,
  },
  {
    key: 'director-pipeline',
    prompt: `${COMMON}
維度：Interaction Director 管線（規格 §6、§6.1、§8.1、§14）。審 apps/interaction-desktop/src/companion/{director.ts,behavior.ts,machine.ts,CompanionApp.tsx,renderer.ts}、rig/timeline.ts、crates/interaction-runtime/src/runtime.rs status()：
Attention/Utility 是否死碼、director.react()/noteFinished() 是否被 app 呼叫、L1 意圖是否繞過 playable()、
quietHours 是否從 runtime status 真的到得了角色（grep status() 輸出鍵）、Fullscreen/勿擾偵測是否存在、
隱藏角色時 receptors 停止而 runtime/tray 正確、bounded queue 是否有界、30fps 降級、reduced-motion 執行中更新、
presentation cancel/clear-all 是否真的清 performing。`,
  },
  {
    key: 'character-protocol',
    prompt: `${COMMON}
維度：角色呈現通用協定（Character Presentation Protocol）。審 apps/interaction-desktop/src/character/**（若不存在則 grep -rn "CharacterAdapter\\|characterId\\|negotiate" apps/interaction-desktop/src crates）、
crates/interaction-core/src/character.rs（若存在）、crates/interaction-runtime/src/presentation.rs、對應測試與 docs/character-protocol/：
manifest 驗證是否可被繞過（路徑穿越、超大 payload、外部 URL／script 自動執行、preferencesSchema 白名單）、
能力協商是否誠實（unsupported 不能演成 exact；fallback 鏈是否會讓 truth state 丟失；純聲音／無視覺角色是否仍能表達 blocked/unknown/claimed/verified）、
intent envelope 的 truthState 是否只能由 Runtime 設定（adapter 能否把 claim-completed 改成 verified-success？）、
messageId 去重／expiresAt 過期不播／cancel 冪等／stale connection generation 的 ack 是否會污染新連線、
adapter crash 時 pending command 是否假裝 completed、receipts 是否誤與工作 verification 混淆、
Emergency／offline／blocked／unknown／waiting-consent 是否能搶占所有非安全演出、
Reduced Motion 協商、custom namespaced channel 是否能影響安全搶占、
小樞 adapter 是否仍有繞過通用接口的私有捷徑（grep Runtime／pages 直接引用耳朵／尾巴／表情名）。`,
  },
  {
    key: 'ia-settings',
    prompt: `${COMMON}
維度：IA 與設定（規格 §12、§13、§15.5、§2 風險分級）。審 apps/interaction-desktop/src/App.tsx、pages/{WorkPage,ConnectPage,MorePage,CompanionPage,SettingsPage,SafetyPage,Onboarding,HomePage,AiPage,ActivityPage}.tsx、styles.css、crates/interaction-runtime/src/activity.rs：
5 入口下所有舊路由是否都可達且高亮正確、相容路由切換（work↔automations、connect↔safety、memory↔activity↔settings）已掛載元件是否真的切換內容、
設定是否真的單一主人（grep 各開關）、estop 觸發/解除路徑完整、
onboarding 3 步 commit 原子性、步驟二 agentChoice 是否真的有效果、「音效預設關閉」有無程式支撐、
Inbox pendingCount 計算順序（truncate 前後）、通知面板鍵盤/焦點、淺色主題 --panel/input 對比、
L0 純呈現動作是否會變成 uncertain receipt 進待決定、一般模式術語外洩（Lease/UUID/raw JSON/Agent Session/Provider/Receptor）、
狀態投影是否 exhaustive（未知原始值是否直接當 UI 標籤）、更換角色後導覽名稱是否更新、過時頁名。`,
  },
  {
    key: 'memory-ui',
    prompt: `${COMMON}
維度：記憶與知識 UI 分層（規格 §11 全段）。審 apps/interaction-desktop/src/pages/{MemoryKnowledgePage,MorePage,KnowledgeAdvanced}.tsx、crates/interaction-runtime/src/{memory,knowledge,curator}.rs：
一般 UI 是否只顯示「關於我的記憶／角色學會的知識／素材與來源」三項、Candidate/Active/Stale/Disputed/Superseded/Knowledge Receipt/Context Bundle 是否移到進階並用規格人類文案、
「角色互動記憶」資料類別是否存在、是否有「不得因一次行為推論人格／不得自動升級為正式知識」的程式規則與測試、後端 10 層是否完好（跑 memory_loop/knowledge_loop/curator_loop）。`,
  },
  {
    key: 'link-transports',
    prompt: `${COMMON}
維度：硬體傳輸誠實性（規格 §9.1 每 Adapter 十項、§15.3）。審 crates/interaction-adapter-declarative/src/{protocol.rs,serial.rs,mqtt.rs,ble.rs,link_caps.rs,lib.rs}、tests/{mqtt_loop,protocol_honesty}.rs、crates/interaction-runtime/src/{hardware,executor,providers}.rs：
ack 逾時絕不重送是否在所有路徑成立、斷線世代與握手競態、wait_for 的 WaitError（TimedOut/Closed/Lagged）是否被呼叫端誠實映射、serial 執行緒生命週期、
mqtt eventloop 重連時 subscribe 遺失、secret 解析會不會進 log/receipt、broadcast lagged 導致 ack 錯配、estop stop_all 未握手時的行為、
health()/status() 是否硬編 healthy、hello.caps 是否用於能力識別、state facts 無 actionId 導致 Observed/Verified 死路、
serial ENOTTY fallback 是否過寬、mqtt_loop 是否真的測了 dedupe/重連/QoS、UI 四態。`,
  },
  {
    key: 'agent-honesty',
    prompt: `${COMMON}
維度：AI 閉環誠實性（規格 §3.2、§8.3、§7.4）。審 crates/interaction-runtime/src/{agents.rs,gateway.rs}、crates/interaction-agent-gateway/src/{lib,claude,codex,codex_exec}.rs、apps/interaction-desktop/src/companion/machine.ts、pages/AiPage.tsx、crates/interaction-api/src/lib.rs scope guard、crates/interaction-cli/src/main.rs：
verify 是否只有 human 路由可達（Tauri command、HTTP scope、CLI）、verified 事件會不會被 agent 自己觸發、resume 是否會放寬 scope、
taxonomy 每一態是否真的會被 emit、程序結束無結果報 failed 還是 unknown、「working」是否早於 fetched、fetched 是否在真的寫入 stdin 後才發、
SSE 重放會不會讓舊 verified 事件重播綠勾、AiPage 訊息是否有刷新、CLI 是否有 resume 入口、dead_code 欄位。`,
  },
  {
    key: 'protocol-conformance',
    prompt: `${COMMON}
維度：裝置線協定一致性（規格 §9）。逐欄比對 crates/interaction-adapter-declarative/src/protocol.rs（Rust 真相）vs
firmware/esp32-companion/esp32-companion.ino vs scripts/esp32-serial-sim.py：
訊息欄位名/型別/kebab-case tag、ack/err 形狀、pair 流程順序、限制值（長度/範圍）、nonce 是否任一方驗證、dedupe ring、stop-all 不需配對、
模擬器覆蓋了 8 周邊中幾個（缺的列出）、韌體硬限制表與 README 一致。不一致＝實機必壞的缺陷。
（韌體已用 arduino-cli 編譯通過；不必重報「無法編譯」。）`,
  },
  {
    key: 'perf-claims',
    prompt: `${COMMON}
維度：效能與量測宣稱（規格 §14、§18-20）。審 docs/acceptance-evidence.md 的效能數字、apps/interaction-desktop/scripts/shu/*.mjs、src/companion/renderer.ts、rig/stage.ts、CompanionApp.tsx pump/rAF：
每個效能數字有無可重現腳本；16–100ms 反應、60fps/30fps 降級、記憶體、bounded queue 有無量測或測試；
rAF 是否無節流每幀重繪透明視窗；每幀配置；interval 洩漏；presentation 指令佇列是否有界；隱藏角色後 rAF/physics/timer/音效是否真的停。
只報有證據的缺口（例如「宣稱數字無產生程式」要 grep 證明）。`,
  },
  {
    key: 'docs-claims',
    prompt: `${COMMON}
維度：文件宣稱 vs 事實。審 CHANGELOG.md [Unreleased] 全部條目、docs/v05-capability-gap-matrix.md、docs/FEATURES.md、docs/DESKTOP-GUIDE.md、README.md、CLAUDE.md、docs/acceptance-evidence.md、docs/character-protocol/*.md、firmware/esp32-companion/README.md、apps/interaction-ios/README.md：
每一句「已完成/已驗證/已有」是否有對應程式碼與測試支撐；模擬器與真機是否處處分清；
測試數字是否與實際套件一致（用 grep/跑測試驗證可疑數字）；有無把「部分」寫成「已有」；版本號與過期敘述。`,
  },
  {
    key: 'safety-invariants',
    prompt: `${COMMON}
維度：安全底線與不變量回歸（規格 §2 十一條、§14、CLAUDE.md）。審 crates/interaction-runtime/src/{runtime,executor,human,sensors,presentation,providers}.rs、crates/interaction-policy、crates/interaction-api/src/lib.rs、crates/interaction-adapter-declarative/src/*.rs、apps/interaction-desktop/src/companion/*、apps/interaction-desktop/src/character/*、src-tauri/src/lib.rs：
estop 重啟後不自動恢復、高風險能力不自動恢復、感測不靜默（所有啟用中的感測是否都在 status/事件/tray/UI）、
Human/Agent/Session token 分離（找可繞過的路由；外部角色 adapter 能否拿到任何 token）、AI 不可授予 consent/解除 estop、dry-run 無副作用、
新程式的 unwrap/expect/blocking sleep/無界 queue/自動重送、secret 進 log、
L0 純呈現不產生干擾性 Receipt UI、L3 硬限制、L4 短效授權是否有 per-use 選項、Emergency 與感測指示是否由可信 host 層保證而非第三方 renderer。`,
  },
  {
    key: 'mobile-server',
    prompt: `${COMMON}
維度：Mobile 伺服器安全（規格 §10、§15.4）。審 crates/interaction-runtime/src/mobile.rs 與 tests/mobile_loop.rs、crates/interaction-api 的 /v1/mobile 守門、CapabilitiesHub MobileSection：
撤銷是否真的關閉現有連線、ack/err/ble.result 有無 authed 守門、配對暴力/DoS、token 撤銷即時性、
未認證連線能否送 observation/ack、pending_acts 洩漏、outbound queue 滿時行為、heartbeat/idle timeout、
多裝置時 send_to_any 的語意、TLS 私鑰檔案權限、agent/session token 是否真的摸不到 /v1/mobile、
mic-level 是否受 consent 與感測不靜默、facts 白名單、act 參數與 iOS App 驗證一致性、mdns 服務名長度、autostart 條件。`,
  },
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
