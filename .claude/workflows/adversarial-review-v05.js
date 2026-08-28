export const meta = {
  name: 'adversarial-review-v05',
  description: 'v0.5 重定位（角色/硬體/AI 三核心）對抗審查：seed claims + 10 維度 find → 獨立懷疑者 verify（high/blocker 雙人）',
  phases: [
    { title: 'Find', detail: '10 個維度並行找缺陷（可帶入 args.seeds 的既有主張）' },
    { title: 'Verify', detail: '每個 finding 由獨立懷疑者反駁；blocker/high 需兩位不同視角皆確認' },
  ],
}

// 用法：
//   Workflow({ scriptPath, args: { seeds: [...], skipDimensions: ['mobile-server'] } })
//   seeds: [{dimension,title,file,line?,severity,claim,evidence}] —— 直接進 Verify（不重找）。
//   skipDimensions: 正在被並行修復、避免對舊碼審查的維度。
//   findModel / verifyModel: 覆寫 finder / verifier 的模型（例如 'opus'），避開單一模型的速率上限。

const ROOT = '/Users/user/Workspace/claude-lab/adaptive-interaction'
const SPEC = '/Users/user/Downloads/adaptive-interaction-v05-core-experience-prompt.md'

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

const COMMON = `你在 ${ROOT}（Rust workspace＋Tauri 桌面＋ESP32 韌體＋iOS 原始碼）。這是 v0.5 產品重定位後的 Phase 7 對抗審查。
規格全文：${SPEC}（需要時用 sed -n 讀相關章節；不要只看標題）。
只回報「真實、可驗證」的缺陷：引用實際檔案與行為證據；不確定就不要報。可以跑 cargo test -p <crate>、pnpm test、grep、sed；不要啟動 daemon、不要跑 e2e/playwright（會撞埠）。
特別留意專案不變量：誠實階梯（queued≠completed≠verified、claim 不冒充 verified、unknown 不演成功）、
實體效果絕不自動重送、感測不靜默、AI 不可自我授權、bounded queues、無 blocking sleep 於 async、
production 不濫用 unwrap()、每項設定只有一個主人、一般 UI 不暴露治理術語、模擬器≠真機。
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
    key: 'ia-settings',
    prompt: `${COMMON}
維度：IA 與設定（規格 §12、§13、§15.5、§2 風險分級）。審 apps/interaction-desktop/src/App.tsx、pages/{WorkPage,ConnectPage,MorePage,CompanionPage,SettingsPage,SafetyPage,Onboarding,HomePage,AiPage,ActivityPage}.tsx、styles.css、crates/interaction-runtime/src/activity.rs：
5 入口下所有舊路由是否都可達且高亮正確、設定是否真的單一主人（grep 各開關）、estop 觸發/解除路徑完整、
onboarding 3 步 commit 原子性、步驟二 agentChoice 是否真的有效果、「音效預設關閉」有無程式支撐、
Inbox pendingCount 計算順序（truncate 前後）、通知面板鍵盤/焦點、淺色主題 --panel/input 對比（截圖 docs/assets/v05-evidence/desktop-inbox.png 可看）、
L0 純呈現動作是否會變成 uncertain receipt 進待決定、一般模式術語外洩（Lease/UUID/raw JSON）、過時頁名。`,
  },
  {
    key: 'memory-ui',
    prompt: `${COMMON}
維度：記憶與知識 UI 分層（規格 §11 全段）。審 apps/interaction-desktop/src/pages/{MemoryKnowledgePage,MorePage,KnowledgeAdvanced}.tsx、crates/interaction-runtime/src/{memory,knowledge,curator}.rs：
一般 UI 是否只顯示「關於我的記憶／小樞學會的知識／素材與來源」三項、Candidate/Active/Stale/Disputed/Superseded/Knowledge Receipt/Context Bundle 是否移到進階並用規格人類文案（等待確認/已採用/可能過期/有不同說法/已被新版取代）、
「角色互動記憶」資料類別是否存在、是否有「不得因一次行為推論人格／不得自動升級為正式知識」的程式規則與測試、後端 10 層是否完好（跑 memory_loop/knowledge_loop/curator_loop）。`,
  },
  {
    key: 'link-transports',
    prompt: `${COMMON}
維度：硬體傳輸誠實性（規格 §9.1 每 Adapter 十項、§15.3）。審 crates/interaction-adapter-declarative/src/{protocol.rs,serial.rs,mqtt.rs,ble.rs,link_caps.rs,lib.rs}、tests/{mqtt_loop,protocol_honesty}.rs、crates/interaction-runtime/src/{hardware,executor,providers}.rs：
ack 逾時絕不重送是否在所有路徑成立、斷線世代與握手競態、serial 執行緒生命週期（drop 後停？fd 洩漏？provider disabled/revoked 後連線是否關）、
mqtt eventloop 重連時 subscribe 遺失、secret 解析會不會進 log/receipt、broadcast lagged 導致 ack 錯配、estop stop_all 未握手時的行為、
health()/status() 是否硬編 healthy（斷線仍健康？）、hello.caps 是否用於能力識別、state facts 無 actionId 導致 Observed/Verified 死路、
serial ENOTTY fallback 是否過寬、mqtt_loop 是否真的測了 dedupe/重連/QoS（找只有註解沒斷言的測試）、UI 四態（只發現/已配對/已測試/已啟用）。`,
  },
  {
    key: 'agent-honesty',
    prompt: `${COMMON}
維度：AI 閉環誠實性（規格 §3.2、§8.3、§7.4）。審 crates/interaction-runtime/src/{agents.rs,gateway.rs}、crates/interaction-agent-gateway/src/{lib,claude,codex,codex_exec}.rs、apps/interaction-desktop/src/companion/machine.ts、pages/AiPage.tsx、crates/interaction-api/src/lib.rs scope guard、crates/interaction-cli/src/main.rs：
verify 是否只有 human 路由可達（Tauri command、HTTP scope、CLI）、verified 事件會不會被 agent 自己觸發、resume 是否會放寬 scope、
taxonomy 11 態每一態是否真的會被 emit（waiting-input？timed-out 在 lease 到期？）、程序結束無結果報 failed 還是 unknown、
「working」是否早於 fetched、fetched 是否在真的寫入 stdin 後才發、SSE 重放會不會讓舊 verified 事件重播綠勾、
AiPage 訊息是否有刷新（approval 300s 自動拒絕是否會被看見）、CLI 是否有 resume 入口、dead_code 欄位。`,
  },
  {
    key: 'protocol-conformance',
    prompt: `${COMMON}
維度：裝置線協定一致性（規格 §9）。逐欄比對 crates/interaction-adapter-declarative/src/protocol.rs（Rust 真相）vs
firmware/esp32-companion/esp32-companion.ino vs scripts/esp32-serial-sim.py：
訊息欄位名/型別/kebab-case tag、ack/err 形狀、pair 流程順序、限制值（長度/範圍）、nonce 是否任一方驗證、dedupe ring、stop-all 不需配對、
模擬器覆蓋了 8 周邊中幾個（缺的列出）、韌體硬限制表與 README 一致。不一致＝實機必壞的缺陷。
（韌體已用 arduino-cli 對 esp32:esp32 3.3.11 編譯通過；不必重報「無法編譯」。）`,
  },
  {
    key: 'perf-claims',
    prompt: `${COMMON}
維度：效能與量測宣稱（規格 §14、§18-20）。審 docs/v05-capability-gap-matrix.md §9 的效能數字、apps/interaction-desktop/scripts/shu/*.mjs、src/companion/renderer.ts、rig/stage.ts、CompanionApp.tsx pump/rAF：
「drawRig 0.452 ms/幀」「全舞台 0.085 ms/幀」有無可重現腳本；16–100ms 反應、60fps/30fps 降級、記憶體、bounded queue 有無量測或測試；
rAF 是否無節流每幀重繪透明視窗；每幀配置；interval 洩漏；presentation 指令佇列是否有界。
只報有證據的缺口（例如「宣稱數字無產生程式」要 grep 證明）。`,
  },
  {
    key: 'docs-claims',
    prompt: `${COMMON}
維度：文件宣稱 vs 事實。審 CHANGELOG.md [Unreleased] 全部 v0.5 條目、docs/v05-capability-gap-matrix.md §9、docs/FEATURES.md v0.5 段、docs/DESKTOP-GUIDE.md、README.md、CLAUDE.md、docs/acceptance-evidence.md、firmware/esp32-companion/README.md、apps/interaction-ios/README.md：
每一句「已完成/已驗證/已有」是否有對應程式碼與測試支撐；模擬器與真機是否處處分清；
測試數字是否與實際套件一致（用 grep/跑測試驗證可疑數字：CLI E2E check 數、vitest 檔數/測試數、Rust 349 的組成）；有無把「部分」寫成「已有」；版本號與過期敘述（CLAUDE.md v0.3.0、README 0.3.0、acceptance-evidence 無 v0.5 段、gap matrix §5 vs §9 矛盾）。
（已知且不必重報：iOS README「沒有 Xcode」已修正；韌體已編譯。）`,
  },
  {
    key: 'safety-invariants',
    prompt: `${COMMON}
維度：安全底線與不變量回歸（規格 §2 十一條、§14、CLAUDE.md）。審 crates/interaction-runtime/src/{runtime,executor,human,sensors,presentation,providers}.rs、crates/interaction-policy、crates/interaction-api/src/lib.rs、crates/interaction-adapter-declarative/src/*.rs、apps/interaction-desktop/src/companion/*、src-tauri/src/lib.rs：
estop 重啟後不自動恢復、高風險能力不自動恢復、感測不靜默（所有啟用中的感測是否都在 status/事件/tray/UI——含 iphone.mic-level）、
Human/Agent/Session token 分離（找可繞過的路由）、AI 不可授予 consent/解除 estop、dry-run 無副作用、
新 v0.5 程式的 unwrap/expect/blocking sleep/無界 queue/自動重送、Rust hit-rect 未 clamp、secret 進 log、
L0 純呈現不產生干擾性 Receipt UI、L3 硬限制、L4 短效授權是否有 per-use 選項。`,
  },
  {
    key: 'mobile-server',
    prompt: `${COMMON}
維度：Mobile 伺服器安全（規格 §10、§15.4）。審 crates/interaction-runtime/src/mobile.rs 與 tests/mobile_loop.rs、crates/interaction-api 的 /v1/mobile 守門、CapabilitiesHub MobileSection：
撤銷是否真的關閉現有連線、ack/err/ble.result 有無 authed 守門、配對暴力/DoS、token 撤銷即時性、
未認證連線能否送 observation/ack、pending_acts 洩漏、outbound queue 滿時行為、heartbeat/idle timeout、
多裝置時 send_to_any 的語意、TLS 私鑰檔案權限、agent/session token 是否真的摸不到 /v1/mobile、
mic-level 是否受 consent 與感測不靜默、facts 白名單、act 參數與 iOS App 驗證（style/title+body/color/on/state）一致性、
mdns 服務名長度（≤15 bytes）、autostart 條件、started 旗標在 bind 失敗後。`,
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

const LENSES = [
  '視角＝可重現性：親自打開檔案、必要時寫最小腳本或跑既有測試證明主張是否成立；若是「缺少功能」主張，grep 全 repo 確認確實不存在。',
  '視角＝規格與誠實階梯：對照規格原文與 CLAUDE.md 不變量，判斷這是否真的違反規格/不變量，還是只是風格偏好或已被其他機制覆蓋。',
]

const verifyOne = (f, dimKey) => {
  const isHot = ['blocker', 'critical', 'high'].includes(String(f.severity))
  const lenses = isHot ? LENSES : [LENSES[0]]
  return parallel(
    lenses.map((lens, i) => () =>
      agent(
        `${COMMON}
你是獨立懷疑者（${lens}）。用力嘗試【反駁】以下 finding（維度 ${dimKey}）：
title: ${f.title}
file: ${f.file}${f.line ? ':' + f.line : ''}
severity: ${f.severity}
claim: ${f.claim}
evidence: ${f.evidence}
親自驗證。只有證據確鑿才 confirmed=true；重複/風格/主觀偏好/已被測試覆蓋的防護/已被修復（檔案可能在你審查時已更新，請以目前檔案內容為準並回報行號漂移）一律 refute。
不確定時 confirmed=false。`,
        { label: `verify${i + 1}:${dimKey}:${String(f.title || '').slice(0, 22)}`, phase: 'Verify', schema: VERDICT_SCHEMA, ...verifyOpts }
      )
    )
  ).then((vs) => {
    const votes = vs.filter(Boolean)
    const confirmed = votes.length > 0 && votes.every((v) => v.confirmed === true && v.corrected_severity !== 'not-a-bug')
    const fixedMeanwhile = !confirmed && votes.length > 0 && votes.some((v) => v.already_fixed === true)
    return { ...f, dimension: dimKey, verdicts: votes, confirmed, fixedMeanwhile, seed: Boolean(f.seed) }
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
    const fresh = (result?.findings ?? []).slice(0, 10).filter((f) => {
      const k = keyOf(f)
      if (seen.has(k)) return false
      seen.add(k)
      return true
    })
    log(`find:${d.key} → ${result?.findings?.length ?? 0} findings, ${fresh.length} new after dedupe`)
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
return {
  totals: { reviewed: all.length, confirmed: confirmed.length, fixedMeanwhile: fixedMeanwhile.length, refuted: refuted.length, unverified: unverified.length, seeds: seeds.length, dimensions: dims.map((d) => d.key) },
  fixedMeanwhile: fixedMeanwhile.map((f) => ({ dimension: f.dimension, seed: f.seed, title: f.title, file: f.file, why: f.verdicts?.map((v) => v.reasoning).join(' || ') })),
  unverified: unverified.map((f) => ({ dimension: f.dimension, seed: f.seed, title: f.title, file: f.file, severity: f.severity, claim: f.claim })),
  confirmed: confirmed.map((f) => ({
    dimension: f.dimension,
    seed: f.seed,
    severity: sevOf(f),
    title: f.title,
    file: f.verdicts?.find((v) => v.corrected_file_line)?.corrected_file_line || `${f.file}${f.line ? ':' + f.line : ''}`,
    claim: f.claim,
    fix_sketch: f.fix_sketch || '',
    regression_test: f.verdicts?.map((v) => v.regression_test_sketch).filter(Boolean).join(' / '),
    reasoning: f.verdicts?.map((v) => v.reasoning).join(' || '),
  })),
  refuted: refuted.map((f) => ({ dimension: f.dimension, seed: f.seed, title: f.title, file: f.file, severity: f.severity, claim: f.claim, why: f.verdicts?.map((v) => v.reasoning).join(' || ') })),
}
