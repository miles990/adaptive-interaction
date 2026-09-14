#!/usr/bin/env python3
import json, os

R = "/private/tmp/claude-501/-Users-user-Workspace-claude-lab-adaptive-interaction/95db6375-b13e-4494-9180-4b15da263657/scratchpad/e2e2/runs/R8"
SRC = "78dcda1a3733c97d266ca9b60ad4461c69ca2032"
BIN = ("/Users/user/Workspace/claude-lab/adaptive-interaction/target/debug/interact-ai, "
       "interact-ai 0.8.0, sha256 1e066d84d28b9397c6c6843522ec6a0fbaf692d10ffbea59481afb79860c84c7")
CLAUDE_CONN = ("crates/interaction-agent-gateway/src/claude.rs @ 78dcda1 (claude -p --input-format stream-json "
               "--output-format stream-json --safe-mode); Claude Code CLI 2.1.263")
CODEX_CONN = ("crates/interaction-agent-gateway/src/codex.rs @ 78dcda1 (codex app-server JSON-RPC v2, "
              "thread/start + thread/resume); Codex CLI 0.153.4")
CLAUDE_WD = R + "/claude/workdir"
CODEX_WD = R + "/codex/workdir"

MCP_DEFECT = {
    "title": "codex 連接器沒有等價於 claude 的 MCP／plugin 封鎖：read-only session 一建立就啟動使用者 ~/.codex 設定裡的 MCP server（chrome-devtools-mcp 瀏覽器控制、codegraph）與 ChatGPT.app computer-use helper，且任務全文被放進 world-readable 的 process 命令列",
    "severity": "high",
    "location": "crates/interaction-agent-gateway/src/codex.rs:166-169（Command::new(&self.binary) … .arg(\"app-server\")，只設 current_dir，無任何 MCP／plugin 限制）對照 crates/interaction-agent-gateway/src/claude.rs:82-84（--strict-mcp-config --mcp-config {\"mcpServers\":{}}）",
    "repro": ("cd " + R + " && unset INTERACT_AI_CLAUDE_BIN INTERACT_AI_CODEX_BIN && "
              "python3 psleak_probe.py \"$PWD/psleak\" 19086   # 建立一個 allowWrite=false／toolScope=[] 的 codex session，"
              "送一個含唯一標記的唯讀任務，同時每 80 ms 掃 `ps -axww -o pid=,ppid=,command=`；"
              "另見 python3 argv_probe.py --agent codex --port 19083 --home \"$PWD/codex/home\" "
              "--workdir \"$PWD/codex/workdir\" --resume-id 01a07b22-d9c6-7470-8812-03a491f9b5c1 --out \"$PWD/argv-codex\""),
    "evidence": [
        R + "/argv-codex/run.log:4 — `codex app-server` 為 daemon(79597) 的子程序",
        R + "/argv-codex/run.log:9 — 子程序 `node /opt/homebrew/bin/npx -y chrome-devtools-mcp@latest`（ppid 79784 = app-server）",
        R + "/argv-codex/run.log:10 — 子程序 `node /opt/homebrew/bin/codegraph serve --mcp`",
        R + "/argv-codex/run.log:11 — 子程序 ChatGPT.app unified-computer-use launch.mjs",
        R + "/argv-codex/run.log:5,18,19,25,26 — 子程序 `/bin/zsh -lc …` 讀取並執行使用者 ~/.zshrc 快照",
        R + "/psleak/raw.json JSON path $.psHits[0].line — SkyComputerUseClient turn-ended 的 argv 內含 \"client\":\"adaptive-interaction\" 與完整 input-messages（含唯一標記 PSLEAKHTYB7B72）",
        R + "/psleak/run.log:3 — 同一筆 PS-HIT 的即時輸出",
        R + "/argv-claude/run.log:5 — 對照組：claude 子程序 argv 內含 --strict-mcp-config --mcp-config {\"mcpServers\":{}}",
    ],
    "phase0MinimalFixCandidate": False,
}

AUDIT_DEFECT = {
    "title": "續開授權檢查（check_resume_not_wider／check_resume_same_workdir）拒絕時完全不留稽核紀錄；被接受的續開也沒有記下「接續了哪一個 provider session」",
    "severity": "medium",
    "location": "crates/interaction-runtime/src/agents.rs:453-468（resume guard 直接 return Err(DomainError::PolicyBlocked)，整段沒有 self.store.audit(...) 呼叫）",
    "repro": ("cd " + R + " && python3 guard_probe.py --agent claude-code --port 19084 --home \"$PWD/claude/home\" "
              "--workdir \"$PWD/claude/workdir\" --resume-id e63b51f8-0c9c-4431-accd-2d9dca4336ba --out \"$PWD/guard-claude\" "
              "；7 個放寬形狀全被 403 拒絕後，GET /v1/audit 裡找不到任何一筆對應紀錄"),
    "evidence": [
        R + "/guard-claude/raw.json JSON path $.auditAfter — 10 次探測（其中 7 次 4xx 拒絕）之後的 audit 內容",
        R + "/claude/raw.json JSON path $.auditTail — 23 筆，kinds 僅 agent-session.capability-issued／agent-session.closed／character.session.truth／character.system-text，無 policy_blocked／resume 相關紀錄",
        R + "/codex/raw.json JSON path $.auditTail — 28 筆，同上",
        "crates/interaction-runtime/src/agents.rs:453-468 — resume guard 無 audit 呼叫（靜態確認）",
    ],
    "phase0MinimalFixCandidate": True,
}

PROJECTION_DEFECT = {
    "title": "session 記錄不保存 resumeProviderSessionId；claude 續開 session 在 create 回應裡 providerSessionId 仍是 null，投影層無法在第一輪結束前顯示「這是某個 thread 的接續」",
    "severity": "low",
    "location": "crates/interaction-runtime/src/agents.rs:547（provider_session_id: None 建檔）與 619-621（等 connector／init 事件回填）；CreateAgentSession.resume_provider_session_id（agents.rs:277）未寫進 AgentSessionRecord",
    "repro": ("cd " + R + " && python3 guard_probe.py --agent claude-code --port 19084 --home \"$PWD/claude/home\" "
              "--workdir \"$PWD/claude/workdir\" --resume-id e63b51f8-0c9c-4431-accd-2d9dca4336ba --out \"$PWD/guard-claude\" "
              "；三個被接受的續開 session，create 回應的 providerSessionId 都是 null"),
    "evidence": [
        R + "/guard-claude/raw.json JSON path $.probes[?(@.name=='ttl-narrower')].record.providerSessionId → null（另兩個被接受的 maxCost-added／workdir-trailing-slash 也是 null）",
        R + "/claude/raw.json JSON path $.s2.createRecord.providerSessionId → null，而 $.cases.S2T1.providerSessionId → \"e63b51f8-0c9c-4431-accd-2d9dca4336ba\"（第一輪之後才回填）",
        R + "/codex/raw.json JSON path $.s2.createRecord.providerSessionId → \"01a07b22-d9c6-7470-8812-03a491f9b5c1\"（codex 在 attach 就拿得到，兩個連接器投影時機不一致）",
    ],
    "phase0MinimalFixCandidate": True,
}

def env(agent, actual_model, source_note, reasoning, new_or_resumed, workdir, authorization, secs, cost):
    return {
        "sourceCommit": SRC,
        "binaryIdentity": BIN,
        "agent": agent,
        "connectorVersion": CLAUDE_CONN if agent.startswith("claude") else CODEX_CONN,
        "requestedModel": "not-specifiable-via-gateway",
        "actualModel": actual_model,
        "actualModelSource": source_note,
        "reasoning": reasoning,
        "newOrResumed": new_or_resumed,
        "workdir": workdir,
        "authorization": authorization,
        "durationSeconds": secs,
        "costOrTokens": cost,
    }

CLAUDE_AUTH = ("runtime: allowWrite=false, toolScope=[], consentScope=[], dataScope=[], ttlMinutes=20, maxMessages=40；"
               "子程序 argv: --safe-mode --permission-mode plan --tools Read,Glob,Grep --strict-mcp-config "
               "--mcp-config {\"mcpServers\":{}}；HTTP 用 <home>/state/api-token（human Bearer）")
CODEX_AUTH = ("runtime: allowWrite=false, toolScope=[], consentScope=[], dataScope=[], ttlMinutes=20, maxMessages=40；"
              "provider 端 turn_context: approval_policy=untrusted, sandbox_policy={\"type\":\"read-only\"}, "
              "permission_profile=managed/restricted(root=read), network=restricted；HTTP 用 <home>/state/api-token（human Bearer）")
CLAUDE_MODEL_SRC = ("provider-local-log (not via gateway) — ~/.claude/projects/"
                    "-private-tmp-claude-501--Users-user-Workspace-claude-lab-adaptive-interaction-95db6375-b13e-4494-9180-4b15da263657-scratchpad-e2e2-runs-R8-claude-workdir/"
                    "e63b51f8-0c9c-4431-accd-2d9dca4336ba.jsonl 的 message.model 欄")
CODEX_MODEL_SRC = ("provider-local-log (not via gateway) — ~/.codex/sessions/2026/09/07/"
                   "rollout-2026-09-07T17-11-16-01a07b22-d9c6-7470-8812-03a491f9b5c1.jsonl 的 turn_context.model 欄")

cases = []

cases.append({
    "id": "K-04-claude-multiturn",
    "scenario": "同一個 agent session（S1）內連續兩個任務：先請 Claude Code 記住一組隨機暗號，再問它暗號是什麼——驗證多輪是否被接受、是否答對、子程序是否重新 spawn",
    "agentOrSurface": "real Claude Code 2.1.263 via HTTP /v1/agent-sessions（daemon 127.0.0.1:19080，隔離 INTERACT_AI_HOME）",
    "evidenceLevel": "real-agent",
    "precondition": ("乾淨隔離 home " + R + "/claude/home（config/interaction.yaml: apiHost 127.0.0.1 / apiPort 19080），"
                     "INTERACT_AI_MOBILE_ADVERTISE=0，INTERACT_AI_CLAUDE_BIN／INTERACT_AI_CODEX_BIN 已 unset；"
                     "唯讀隔離 workdir " + CLAUDE_WD + " 內只有一個 NOTES.md；暗號 X55XY4F8 於執行時隨機產生，未寫進任何檔案"),
    "steps": [
        "spawn daemon：INTERACT_AI_HOME=<out>/claude/home target/debug/interact-ai serve（pid 33157），等 /v1/ready",
        "開 SSE：GET /v1/events（Bearer human token）全程錄到 claude/sse.jsonl",
        "POST /v1/agent-sessions {agentId:\"claude-code\", workdir:<isolated>, ttlMinutes:20, maxMessages:40, allowWrite:false, toolScope:[], consentScope:[], dataScope:[]} → asession-de74fc7c-c24a-4e57-b05f-a5d036a33d96",
        "記錄此刻 daemon 的 agent 子程序（ppid 鏈屬於 33157 者）",
        "POST …/messages {kind:\"task\", body:{task:\"請記住暗號 X55XY4F8，只回覆「已記住」。不要使用任何工具，不要讀寫任何檔案。\"}}",
        "每 0.5 s GET /v1/agent-sessions/{id} 直到終態，同時記錄子程序清單",
        "在同一個 session 上 POST …/messages {kind:\"task\", body:{task:\"我剛才要你記住的暗號是什麼？只回暗號，不要其他文字。\"}}",
        "再輪詢到終態，GET …/messages?direction=from-session 取回答，並比對子程序 pid 與 provider 端 jsonl",
    ],
    "expected": {
        "ui": "第二個任務回 200 被收下；SSE 再走一次 fetched → working → claimed-completed；GET /v1/agent-sessions/{id} 的 state 回到 claimed-completed",
        "coreState": "同一個 session record：state=claimed-completed、providerSessionId 不變、budget.spentMessages 增加、budget.spentCost 累加",
        "effect": "同一個 claude 子程序被重複使用（pid 不變、不重新 spawn），provider 端 jsonl 在同一個 session id 下多出第二輪，且回答等於第一輪要求記住的暗號",
    },
    "result": "completed",
    "actual": ("三個投影全部對上。第二個任務 POST 回 200（stateBefore=claimed-completed，is_open() 含 ClaimedCompleted）；"
               "SSE 記到 fetched 09:11:54.964Z → working 09:11:57.363Z → claimed-completed 09:11:57.374Z；"
               "from-session 第二則 result.summary = \"X55XY4F8\"（完全正確）、costUsd 0.14706625；"
               "session record spentMessages=4、spentCost=0.27653525（= 0.129469 + 0.14706625）、providerSessionId 兩輪都是 "
               "e63b51f8-0c9c-4431-accd-2d9dca4336ba；子程序 pid 33355 在兩輪前後完全相同 —— claude stream-json 連接器"
               "**沒有**為第二輪重新 spawn，而是寫進同一支子程序的 stdin；provider 端 "
               "e63b51f8-0c9c-4431-accd-2d9dca4336ba.jsonl 只有一個檔案，第 11 行（user，permissionMode=plan）／第 13 行"
               "（assistant，model=claude-fable-5-1，內容 X55XY4F8）就是第二輪。"),
    "evidence": [
        R + "/claude/run.log:4,5,7 — S1T1/send 200、S1T1/state claimed-completed（mono 7.171）、S1T2/send 200（sessionState=claimed-completed）",
        R + "/claude/raw.json JSON path $.cases.S1T2.fromSession[1].body → {\"costUsd\":0.14706625,\"summary\":\"X55XY4F8\"}",
        R + "/claude/raw.json JSON path $.cases.S1T2.childrenBefore[0].pid = 33355 與 $.cases.S1T2.childrenAfter[0].pid = 33355",
        R + "/claude/raw.json JSON path $.s1.finalRecord.budget → {\"maxCost\":0.0,\"maxDurationMs\":1200000,\"maxMessages\":40,\"spentCost\":0.27653525,\"spentMessages\":4}",
        R + "/claude/sse.jsonl — agent.session.state fetched 2026-09-07T09:11:54.964210Z / working 09:11:57.363359Z / claimed-completed 09:11:57.374044Z",
        "~/.claude/projects/-private-tmp-claude-501--Users-user-Workspace-claude-lab-adaptive-interaction-95db6375-b13e-4494-9180-4b15da263657-scratchpad-e2e2-runs-R8-claude-workdir/e63b51f8-0c9c-4431-accd-2d9dca4336ba.jsonl:11,13 — 第二輪 user（permissionMode=plan）與 assistant（model=claude-fable-5-1，text=X55XY4F8）",
    ],
    "observations": [
        "claude 連接器多輪不重新 spawn：同一支 `claude -p --input-format stream-json` 子程序（pid 33355）跨兩輪存活，訊息經 stdin 送入（claude.rs:323 send_user_message）。",
        "REST 狀態投影對快輪幾乎不可觀測：第二輪的 active 視窗只有約 11 ms（SSE working 09:11:57.363 → claimed-completed 09:11:57.374），0.5 s 的 GET /v1/agent-sessions/{id} 輪詢**一次都沒**看到 active（$.cases.S1T2.transitions = []）。要判斷「第二個任務有沒有被接走」必須看 SSE 或 mailbox，不能靠輪詢 state。",
        "第一輪 assistant 有 259 個 thinking token，第二／三輪為 0（provider log usage.output_tokens_details.thinking_tokens）。",
        "provider log 結尾多一筆 {\"type\":\"mode\",\"mode\":\"normal\"}（jsonl:21），但每一輪 user 記錄的 permissionMode 都是 plan——實際套用的是 plan，那筆 mode 記錄不是本次 turn 的授權事實。",
    ],
    "productDefects": [],
    "limitations": ("harness 的等待迴圈在沒觀察到 active 時會固定等 60 s 才收尾，所以 $.cases.*.elapsed（60.2）不是真實 turn 時間；"
                    "真實 turn 時間以 SSE 時戳與 transitions 為準。只跑了 2 輪，沒有測 maxMessages 邊界或第 3 輪以上。"),
    "cleanup": ("session 已 POST /close（reason=\"K-04 close before resume\"）；daemon pid 33157 由 harness SIGTERM 結束（returncode -15），"
                "已確認 pid 已 dead、19080 埠已釋放、無殘留 `claude -p --input-format` 子程序；未寫入 ~/.adaptive-interaction／~/.claude／~/.codex（provider log 只讀）。"),
    "environment": env("claude-code", "claude-fable-5-1", CLAUDE_MODEL_SRC,
                       "gateway 無法指定或回報 reasoning 設定；provider log 顯示第一輪 thinking_tokens=259、第二輪 0；permissionMode=plan",
                       "new", CLAUDE_WD, CLAUDE_AUTH, 62.8,
                       "USD 0.27653525（turn1 0.129469 + turn2 0.14706625，取自 from-session result.costUsd 與 record.budget.spentCost）"),
})

cases.append({
    "id": "K-04-claude-resume",
    "scenario": "關閉 S1 後（確認 mailbox 409），用 resumeProviderSessionId=S1 的 providerSessionId 建立 S2，問「上一個 session 要你記住的暗號」——驗證是否真的走 claude --resume 原生續接",
    "agentOrSurface": "real Claude Code 2.1.263 via HTTP /v1/agent-sessions（daemon 127.0.0.1:19080；另一次在 19082 重啟後續接）",
    "evidenceLevel": "real-agent",
    "precondition": "K-04-claude-multiturn 的 S1 已 claimed-completed，providerSessionId=e63b51f8-0c9c-4431-accd-2d9dca4336ba，resolvedWorkdir=" + CLAUDE_WD,
    "steps": [
        "POST /v1/agent-sessions/asession-de74fc7c…/close {reason:\"K-04 close before resume\"}",
        "對已關閉的 session 再 POST …/messages（預期 409）",
        "POST /v1/agent-sessions {agentId:\"claude-code\", workdir:<同一個>, ttlMinutes:20, maxMessages:40, allowWrite:false, toolScope:[], consentScope:[], dataScope:[], resumeProviderSessionId:\"e63b51f8-…\"}",
        "POST …/messages {kind:\"task\", body:{task:\"我在上一個 session 要你記住的暗號是什麼？只回暗號，不要其他文字。\"}}，輪詢到終態並取回答",
        "檢查 S2 的 providerSessionId 是否等於 S1；檢查 provider 端 jsonl 是否同一個檔案、續接的那一輪 prompt 有沒有夾帶暗號（排除 memory contextBundle 洩題）",
        "另起 argv_probe.py：在同一個 home 重新啟動 daemon（port 19082），再續接一次，並以 50 ms 間隔掃 ps 取子程序完整 argv",
    ],
    "expected": {
        "ui": "close 後 POST message 回 409；S2 create 回 200，任務走到 claimed-completed",
        "coreState": "S2 是獨立 session record（新 sessionId、獨立 budget），providerSessionId 指向同一個 provider thread",
        "effect": "子程序 argv 含 --resume <S1 providerSessionId>；provider 端 jsonl 續寫同一個檔案；回答等於 S1 記住的暗號，而該暗號沒有出現在 S2 的 prompt 裡",
    },
    "result": "completed",
    "actual": ("三個投影全部對上，且排除了洩題。close 後 POST message 回 409 conflict（\"agent session … is Closed; mailbox closed\"）；"
               "S2 create 200（asession-c9d37a4e-86b9-49c0-ab39-9d64ee66443e），3.54 s 後 claimed-completed，"
               "from-session result.summary = \"X55XY4F8\"（正確）、costUsd 0.01802475；S2 的 providerSessionId 回填後等於 S1 的 "
               "e63b51f8-0c9c-4431-accd-2d9dca4336ba（Claude Code 2.1.263 在 --resume 時沿用同一個 session id，沒有換 id）；"
               "provider 端 project 目錄自始至終只有一個 jsonl，續接那一輪寫在同一檔的第 17（user）／19（assistant，X55XY4F8）行。"
               "洩題排除：第 17 行 user 內容不含 X55XY4F8（只有 task-memory 摘要，含 goal/costUsd/providerSessionId，無暗號）。"
               "直接 argv 證據：`claude -p --input-format stream-json --output-format stream-json --verbose --safe-mode "
               "--strict-mcp-config --mcp-config {\"mcpServers\":{}} --permission-mode plan --tools Read,Glob,Grep "
               "--resume e63b51f8-0c9c-4431-accd-2d9dca4336ba` —— 續接時授權旗標（--safe-mode／plan／唯讀工具集）被重新上鎖，"
               "不是繼承。daemon 重啟後（新 port 19082、同一個 home）續接仍成功並答對，代表授權比對用的是持久化的 session record。"),
    "evidence": [
        R + "/claude/run.log:9,10,11 — S1/close 200 state=closed、S1/post-after-close 409 conflict、S2/create 200",
        R + "/claude/raw.json JSON path $.s1.postAfterClose → {\"status\":409,…\"conflict: agent session … is Closed; mailbox closed\"}",
        R + "/claude/raw.json JSON path $.cases.S2T1.fromSession[0].body → {\"costUsd\":0.01802475,\"summary\":\"X55XY4F8\"}",
        R + "/claude/raw.json JSON path $.s1.providerSessionId 與 $.s2.providerSessionId 皆為 e63b51f8-0c9c-4431-accd-2d9dca4336ba",
        R + "/argv-claude/run.log:5 — 子程序完整 argv 含 `--resume e63b51f8-0c9c-4431-accd-2d9dca4336ba`",
        R + "/argv-claude/raw.json JSON path $.fromSession[0].body.summary = \"X55XY4F8\"、$.finalRecord.providerSessionId = e63b51f8-…、$.sessionsAfterRestart（重啟後兩筆 closed 記錄都留著 providerSessionId）",
        "~/.claude/projects/…-R8-claude-workdir/e63b51f8-0c9c-4431-accd-2d9dca4336ba.jsonl:17,19 — 續接輪的 user（不含暗號）與 assistant（X55XY4F8）",
        "crates/interaction-agent-gateway/src/claude.rs:106-107 — spec.resume_provider_session ⇒ args.extend([\"--resume\", …])（靜態對照）",
    ],
    "observations": [
        "Claude Code 2.1.263 的 --resume 沿用同一個 provider session id 並續寫同一個 jsonl（不是 fork 出新 id）——所以「S2 的 providerSessionId 是否等於 S1」在這個版本答案是「等於」。",
        "create 回應當下 S2 的 providerSessionId 仍是 null，要等第一輪 init 事件才回填（見 productDefects 的投影缺口）。",
        "續接輪的 cache_read_input_tokens = 9259（第一輪只有 2800），與「provider 端把前面的對話當前綴重讀」一致。",
        "續接同時做了「daemon 重啟後」的變體（argv_probe，port 19082）：重啟後 S1 記錄仍在（state=closed），resume 授權比對通過、答案仍正確。",
    ],
    "productDefects": [PROJECTION_DEFECT],
    "limitations": ("只證明了 Claude Code 2.1.263 的行為；「provider 換不換 session id」是 CLI 版本相依的，換版本要重測。"
                    "沒有測試跨 workdir 專案目錄的 resume（那條路徑被 check_resume_same_workdir 擋掉，見 K-04-resume-not-wider）。"),
    "cleanup": ("S2 已 POST /close（reason=\"K-04 done\"）；argv probe 的 session 也已 close；daemon 33157／76108 皆已 SIGTERM 結束，"
                "19080／19082 埠已釋放，`ps` 確認無殘留 claude 子程序。"),
    "environment": env("claude-code", "claude-fable-5-1", CLAUDE_MODEL_SRC,
                       "gateway 無法指定或回報；續接輪 thinking_tokens=0，permissionMode=plan（argv 顯式 --permission-mode plan）",
                       "resumed（resumeProviderSessionId=e63b51f8-0c9c-4431-accd-2d9dca4336ba）", CLAUDE_WD, CLAUDE_AUTH, 5.94,
                       "USD 0.01802475（S2 turn）＋ USD 0.02325525（重啟後 argv probe turn）= 0.041280"),
})

cases.append({
    "id": "K-04-codex-multiturn",
    "scenario": "同一個 codex session（S1）內連續兩個任務：先記住隨機暗號，再問暗號——驗證多輪與子程序重用",
    "agentOrSurface": "real Codex CLI 0.153.4（codex app-server）via HTTP /v1/agent-sessions（daemon 127.0.0.1:19081，隔離 INTERACT_AI_HOME）",
    "evidenceLevel": "real-agent",
    "precondition": ("乾淨隔離 home " + R + "/codex/home（apiPort 19081），INTERACT_AI_MOBILE_ADVERTISE=0，"
                     "INTERACT_AI_CLAUDE_BIN／INTERACT_AI_CODEX_BIN 已 unset；唯讀隔離 workdir " + CODEX_WD + "；"
                     "harness 帶 --approve approve（若出現 waiting-for-consent 會由 harness 代為核可，實際上沒有觸發）；暗號 J8SZQ555 執行時隨機產生"),
    "steps": [
        "spawn daemon（pid 35619），等 /v1/ready，開 SSE",
        "POST /v1/agent-sessions {agentId:\"codex\", workdir:<isolated>, ttlMinutes:20, maxMessages:40, allowWrite:false, toolScope:[], consentScope:[], dataScope:[]} → asession-89462e90-f08d-48f7-95cd-7eb1b40fb879",
        "POST …/messages 任務一：「請記住暗號 J8SZQ555，只回覆「已記住」。不要使用任何工具，不要讀寫任何檔案。」",
        "輪詢到終態；記錄子程序",
        "同一 session POST …/messages 任務二：「我剛才要你記住的暗號是什麼？只回暗號，不要其他文字。」",
        "輪詢到終態，取 from-session，比對子程序 pid 與 ~/.codex rollout 檔",
    ],
    "expected": {
        "ui": "第二個任務 200；SSE 再走一次 fetched → working → claimed-completed",
        "coreState": "同一 session record，providerSessionId（thread id）不變，spentMessages 增加",
        "effect": "同一支 codex app-server 子程序被重用；rollout jsonl 同一檔多出第二個 turn；回答等於暗號",
    },
    "result": "completed",
    "actual": ("三個投影全部對上。任務一 6.26 s 後 claimed-completed，result.summary=\"已記住\"；任務二 POST 200"
               "（stateBefore=claimed-completed），4.17 s 後 claimed-completed，result.summary=\"J8SZQ555\"（正確）；"
               "SSE：fetched 09:11:23.412Z → working 09:11:23.439Z → claimed-completed 09:11:27.369Z；"
               "子程序 `codex app-server` pid 35908 兩輪前後不變（未重新 spawn）；providerSessionId 兩輪都是 "
               "01a07b22-d9c6-7470-8812-03a491f9b5c1；rollout 檔 rollout-2026-09-07T17-11-16-01a07b22-….jsonl 第 18 行"
               "（turn_context，第二個 turn_id）／第 21 行（AgentMessage \"J8SZQ555\"）／第 25 行（task_complete，duration_ms 3935）。"
               "全程沒有出現 waiting-for-consent，harness 的 --approve approve 一次都沒用到（$.cases.*.approvals = null）。"),
    "evidence": [
        R + "/codex/run.log:5-11 — S1T1 active→claimed-completed（5,6）、S1T2/send 200（8）、S1T2 active→claimed-completed（9,10）",
        R + "/codex/raw.json JSON path $.cases.S1T2.fromSession[1].body → {\"summary\":\"J8SZQ555\"}",
        R + "/codex/raw.json JSON path $.cases.S1T2.childrenBefore[0].pid = 35908 = $.cases.S1T2.childrenAfter[0].pid",
        R + "/codex/raw.json JSON path $.s1.finalRecord.budget → spentMessages 4、spentCost 0.0",
        R + "/codex/sse.jsonl — agent.session.state fetched 09:11:23.412959Z / working 09:11:23.439066Z / claimed-completed 09:11:27.369429Z",
        "~/.codex/sessions/2026/09/07/rollout-2026-09-07T17-11-16-01a07b22-d9c6-7470-8812-03a491f9b5c1.jsonl:18,21,25 — 第二個 turn_context／AgentMessage J8SZQ555／task_complete",
    ],
    "observations": [
        "codex 連接器多輪同樣不重新 spawn：同一支 `codex app-server`（pid 35908）跨兩輪存活，第二輪走 JSON-RPC 送進同一個 thread。",
        "codex 完全不回報 USD 成本：兩輪的 result body 沒有 costUsd 欄位、record.budget.spentCost 一直是 0.0。runtime 對此是誠實的——gateway.rs:297-302 在 maxCost>0 時直接以 400 拒絕建立 codex session（實測見 K-04-resume-not-wider 的 maxCost-added 形狀）。",
        "provider log 的 token_count 累計值在同一支 app-server 內是累加的（turn1 total 24917 → turn2 total 55195），但換一支 app-server 續接時會從頭算（見 K-04-codex-resume）。",
        "本次任務沒有觸發任何工具呼叫，所以 codex 的 untrusted 核可路徑沒有被走到——這一輪不能拿來當「核可流程可用」的證據。",
    ],
    "productDefects": [MCP_DEFECT],
    "limitations": ("只有 2 輪；沒有測 codex 在多輪中途被 interrupt／approve 的行為。costUsd 一律 unknown（連接器不提供），"
                    "token 數只能從 provider 端 rollout 唯讀取得，gateway 不投影。"),
    "cleanup": ("session 已 close；daemon pid 35619 SIGTERM 結束（returncode -15）；close 後子程序清單為空"
                "（$.s1.childrenAfterClose = []），19081 埠已釋放。"),
    "environment": env("codex", "gpt-6-astra", CODEX_MODEL_SRC,
                       "provider log turn_context: effort=\"medium\"、collaboration_mode.settings.reasoning_effort=\"medium\"、summary=\"auto\"；gateway 無法指定也不回報",
                       "new", CODEX_WD, CODEX_AUTH, 10.66,
                       "USD unknown（codex 連接器不回報成本，record.budget.spentCost = 0.0）；provider log token_count：turn1 total 24917 tokens、turn2 累計 55195（last 30278）"),
})

cases.append({
    "id": "K-04-codex-resume",
    "scenario": "關閉 codex S1 後（確認 mailbox 409），用 resumeProviderSessionId 建立 S2 問暗號——驗證 codex thread/resume 原生續接與授權是否重新上鎖",
    "agentOrSurface": "real Codex CLI 0.153.4（codex app-server thread/resume）via HTTP（daemon 19081；另一次在 19083 重啟後續接）",
    "evidenceLevel": "real-agent",
    "precondition": "K-04-codex-multiturn 的 S1 已 claimed-completed，providerSessionId=01a07b22-d9c6-7470-8812-03a491f9b5c1，resolvedWorkdir=" + CODEX_WD,
    "steps": [
        "POST /v1/agent-sessions/asession-89462e90…/close {reason:\"K-04 close before resume\"}",
        "對已關閉 session POST …/messages（預期 409）",
        "POST /v1/agent-sessions {…, resumeProviderSessionId:\"01a07b22-d9c6-7470-8812-03a491f9b5c1\", workdir 同一個, ttlMinutes:20, maxMessages:40, allowWrite:false}",
        "POST …/messages 問「上一個 session 要你記住的暗號」，輪詢到終態取回答",
        "比對 S2 的 providerSessionId、rollout 檔是否同一個、續接輪 prompt 有無夾帶暗號、turn_context 的 sandbox/approval 是否仍是唯讀",
        "另起 argv_probe.py：同一個 home 重啟 daemon（port 19083）再續接一次，並掃子程序 argv",
    ],
    "expected": {
        "ui": "close 後 POST message 409；S2 create 200 並走到 claimed-completed",
        "coreState": "S2 為新 session record，providerSessionId 指向同一 thread",
        "effect": "rollout jsonl 續寫同一檔；turn_context 仍是 read-only／untrusted；回答等於 S1 的暗號，且該暗號沒出現在 S2 prompt",
    },
    "result": "completed",
    "actual": ("三個投影全部對上並排除洩題。close 後 POST message 409 conflict；S2 create 200"
               "（asession-05e463ff-093a-438f-b944-ac3eb7007773），create 當下 providerSessionId 就已是 "
               "01a07b22-d9c6-7470-8812-03a491f9b5c1（codex attach 立刻拿得到，和 claude 不同）；6.18 s 後 claimed-completed，"
               "result.summary=\"J8SZQ555\"（正確）；rollout 仍是同一個檔案 rollout-2026-09-07T17-11-16-01a07b22-….jsonl，"
               "續接輪寫在第 29（turn_context）／32（AgentMessage J8SZQ555）／36（task_complete）行。"
               "授權重新上鎖：三個 turn_context（含續接輪）的 approval_policy 都是 \"untrusted\"、sandbox_policy 都是 "
               "{\"type\":\"read-only\"}、permission_profile 都是 managed/restricted(root=read)、network=\"restricted\"、"
               "model 都是 gpt-6-astra。洩題排除：續接輪的 user message（rollout:30）不含 J8SZQ555，只有 task-memory 摘要。"
               "重啟 daemon（port 19083）後再續接一次同樣答對，rollout 續寫到第 44 行。"),
    "evidence": [
        R + "/codex/run.log:12,13,14,17,18 — S1/close 200、post-after-close 409、S2/create 200、S2T1 claimed-completed",
        R + "/codex/raw.json JSON path $.s1.postAfterClose.status = 409、$.s2.createRecord.providerSessionId = \"01a07b22-d9c6-7470-8812-03a491f9b5c1\"",
        R + "/codex/raw.json JSON path $.cases.S2T1.fromSession[0].body → {\"summary\":\"J8SZQ555\"}",
        "~/.codex/sessions/2026/09/07/rollout-2026-09-07T17-11-16-01a07b22-d9c6-7470-8812-03a491f9b5c1.jsonl:29,30,32,36 — 續接輪 turn_context（read-only／untrusted／gpt-6-astra）、不含暗號的 user message、AgentMessage J8SZQ555、task_complete",
        R + "/argv-codex/raw.json JSON path $.fromSession[0].body.summary = \"J8SZQ555\"、$.finalRecord.providerSessionId = 01a07b22-…（重啟後續接）",
        R + "/argv-codex/run.log:43 — SkyComputerUseClient turn-ended argv 內的 \"thread-id\":\"01a07b22-d9c6-7470-8812-03a491f9b5c1\"，佐證續接輪跑在同一個 thread",
        "crates/interaction-agent-gateway/src/codex.rs:321-330 — resume ⇒ thread/resume（靜態對照）；crates/interaction-runtime/src/gateway.rs:335-337 — 續開時旗標重新上鎖的註解與程式",
    ],
    "observations": [
        "codex 的 thread id 續接後不變，且 rollout 檔案續寫同一個——是真正的原生 thread 續接，不是重放。",
        "codex 在 attach 當下就回報 providerSessionId（thread/resume 的回應），claude 要等 init 事件才回填——兩個連接器的投影時機不一致。",
        "token 累計在續接後歸零重算（同檔 rollout:24 的 total 55195 → rollout:35 的 total 35609），因為換了一支 app-server 程序；跨 session 的累計成本/用量沒有任何一層在彙總。",
        "續接輪的 workspace_roots 除了 session workdir，還被 codex 自己的設定加進 /Users/user/.agents/skills/agmsg/{db,teams,run} 三個目錄（rollout turn_context.workspace_roots）——runtime 宣告的 workdir 不是 codex 實際的 workspace 邊界（sandbox 仍是 read-only，故本輪未造成寫入）。",
    ],
    "productDefects": [],
    "limitations": ("沒有測試 codex 在 resume 時如果 provider 端 thread 已被刪除／過期會怎樣；沒有測跨 workdir 的 codex resume"
                    "（被 runtime 擋在前面）。codex 的 approval（untrusted）路徑本輪未觸發，續接後的核可行為未驗證。"),
    "cleanup": "S2 與 argv probe 的 session 皆已 close；daemon 35619／79597 已 SIGTERM 結束；19081／19083 埠已釋放；`ps` 確認無殘留 `codex app-server`。",
    "environment": env("codex", "gpt-6-astra", CODEX_MODEL_SRC,
                       "provider log turn_context（含續接輪）: effort=\"medium\"、reasoning_effort=\"medium\"、summary=\"auto\"",
                       "resumed（resumeProviderSessionId=01a07b22-d9c6-7470-8812-03a491f9b5c1）", CODEX_WD, CODEX_AUTH, 8.75,
                       "USD unknown（連接器不回報）；provider log token_count：續接輪 total 35618 tokens、重啟後續接輪 total 36288"),
})

not_wider_steps = [
    "沿用同一個 daemon 與同一個已關閉的 S1（providerSessionId 已知、resolvedWorkdir 已落地）",
    "對每一種放寬形狀 POST /v1/agent-sessions（只 create，不送任務），記錄 HTTP 狀態與錯誤訊息",
    "形狀（k04.py，claude 與 codex 各跑一輪）：allowWrite=true+toolScope[workspace.write]+consentScope[…]／換 workdir／省略 workdir／ttlMinutes 120／maxMessages 200／同時省略 ttlMinutes 與 maxMessages",
    "形狀（guard_probe.py，claude 與 codex 各跑一輪，補齊「哪一道關卡先擋」與「縮小是否放行」）：allowWrite 無 scope／allowWrite 只帶 consent／allowWrite 全帶／dataScope 加 domain:private／maxCost 5.0／workdir 用 ../ 繞路／workdir 加結尾斜線／ttl 與 maxMessages 都縮小／resumeProviderSessionId 空字串／只有空白",
    "任何被接受的 create 立刻 POST /close 收尾，並記錄子程序",
]

cases.append({
    "id": "K-04-resume-not-wider",
    "scenario": "續開不得放寬授權：對已知的 providerSessionId 送出各種「比上一次更寬」的 create 請求，實測 check_resume_not_wider／check_resume_same_workdir 是否確定性擋下；同時確認「縮小」不會被誤擋",
    "agentOrSurface": "HTTP /v1/agent-sessions（daemon 19080/19081 的 k04.py 內嵌探測 + 19084/19085 的 guard_probe.py）；被拒絕的請求不會 spawn 任何 agent 子程序",
    "evidenceLevel": "integration",
    "precondition": ("claude 側 S1 providerSessionId=e63b51f8-0c9c-4431-accd-2d9dca4336ba、budget{ttl 20 分鐘, maxMessages 40, maxCost 0}、"
                     "allowWrite=false、toolScope=[]、consentScope=[]、dataScope=[]、resolvedWorkdir=" + CLAUDE_WD + "；"
                     "codex 側 S1 providerSessionId=01a07b22-d9c6-7470-8812-03a491f9b5c1、同樣的 budget／scope，resolvedWorkdir=" + CODEX_WD),
    "steps": not_wider_steps,
    "expected": {
        "ui": "每一個放寬形狀都回 4xx，錯誤訊息指名放寬了哪一項；縮小的形狀回 200",
        "coreState": "被拒絕時不產生任何 session record（GET /v1/agent-sessions 不多出項目）",
        "effect": "被拒絕時不 spawn 任何 claude/codex 子程序，不對 provider 端造成任何一輪呼叫",
    },
    "result": "correctly-blocked",
    "actual": ("兩個 agent 各 6 個放寬形狀（k04.py）全部 403 policy_blocked，錯誤訊息逐項指名："
               "可用工具（workspace.write 不在上次授權範圍）／更換工作目錄（訊息帶出上次的絕對路徑）／必須帶同一個工作目錄"
               "（省略＝系統另挑資料夾）／時間上限（上次 20 分鐘、這次 120）／訊息上限（上次 40、這次 200）／同時省略 ttl 與 "
               "maxMessages 時落到 runtime 預設 120 分鐘一樣被擋。guard_probe 再補 10 個形狀，兩個 agent 結果一致：\n"
               "  allowWrite 無 scope → 403 consent required（write-enabled session 缺少 toolScope workspace.write，寫入前置檢查先擋）\n"
               "  allowWrite 只帶 consentScope → 403 同上\n"
               "  allowWrite 全帶 → 403 policy blocked（可用工具放寬）\n"
               "  dataScope 加 domain:private → 403 policy blocked（資料範圍放寬）\n"
               "  maxCost 5.0 → claude 200 ACCEPTED（原本沒有金額上限，加上限是縮小，正確放行）／codex 400 validation "
               "（codex 不回報 USD 成本，maxCost 無法強制執行——誠實拒絕）\n"
               "  workdir 用 `<workdir>/../workdir-other` → 403（canonicalize 後比對，字串前綴繞不過去）\n"
               "  workdir 加結尾斜線 → 200（正規化後同一路徑，正確放行）\n"
               "  ttlMinutes 5 + maxMessages 10（都縮小）→ 200（正確放行）\n"
               "  resumeProviderSessionId=\"\" 或全空白 → 403 policy blocked（找不到上一次的授權紀錄）\n"
               "被拒絕的請求都在 create 階段就返回，SSE 沒有任何 session.started，`ps` 也沒有對應子程序。"),
    "evidence": [
        R + "/claude/run.log:16-21 — claude 側 6 個放寬形狀全 403 及各自的訊息",
        R + "/codex/run.log:20-25 — codex 側 6 個放寬形狀全 403 及各自的訊息",
        R + "/guard-claude/run.log:1-10 — claude 側 10 個形狀的狀態碼與訊息（maxCost-added=200、workdir-trailing-slash=200、ttl-narrower=200，其餘 403）",
        R + "/guard-codex/run.log:1-10 — codex 側同 10 個形狀（maxCost-added=400 validation，其餘同 claude）",
        R + "/claude/raw.json JSON path $.probes[*].status → 全為 403；$.probes[*].resp.error.code → policy_blocked",
        R + "/guard-claude/raw.json JSON path $.probes[?(@.name=='workdir-dotdot')].error.message → \"接續上次的工作不得更換工作目錄（上次是 …/claude/workdir）\"",
        R + "/claude/sse.jsonl — 探測期間（09:13:57.842–847Z）沒有任何 session.started 事件",
        "crates/interaction-runtime/src/agents.rs:105-176（check_resume_not_wider）與 195-237（check_resume_same_workdir）— 靜態對照",
    ],
    "observations": [
        "`if input.allow_write && !original.allow_write`（agents.rs:171-175，ConsentRequired）在「原 session 是唯讀且 consentScope 為空」時走不到：任何能通過寫入前置檢查的 payload 一定帶 consentScope=[\"agent-session:workspace-write\"]／toolScope=[\"workspace.write\"]，會先被 scope 的 extra 檢查擋掉。三道關卡是層層防禦，對外行為都是 4xx。",
        "「省略欄位」被當成放寬處理，符合註解宣稱（省略 → runtime 預設 ttl 120 分鐘 → 比上次寬 → 擋）。",
        "workdir 正規化確實有效：`A/../B` 被擋、結尾斜線被視為同一路徑放行。",
        "guard_probe 中被接受的 3 個縮小形狀真的 attach 了子程序（claude 有 3 次、codex 有 2 次 spawn），但沒送任何任務，所以沒有模型呼叫、沒有費用；全部立刻 POST /close 收尾。",
    ],
    "productDefects": [AUDIT_DEFECT],
    "limitations": ("沒有測跨 agent 的續接（用 claude 的 thread id 去續接 codex，或反之）——本次兩個 agent 用不同的隔離 home，"
                    "同一個 home 內同時有兩種 agent 記錄的情境未涵蓋。也沒有測 delegation envelope 與 resume 的交互作用。"),
    "cleanup": "guard_probe 中被接受的 session 全部 POST /close（狀態 closed）；daemon 19084／19085 已 SIGTERM 結束；埠已釋放；`ps` 確認無殘留子程序。",
    "environment": env("claude-code + codex（兩邊各跑一輪相同形狀）", "n/a（所有放寬請求都在 create 階段被拒，沒有任何 provider turn；被接受的縮小形狀也沒送任務）",
                       "provider-local-log (not via gateway) — 本案未產生新的 provider turn，故無 model 欄可讀",
                       "n/a（無模型呼叫）", "resumed（每一次 create 都帶 resumeProviderSessionId）",
                       CLAUDE_WD + " 與 " + CODEX_WD + "（另含刻意換到 workdir-other 與 ../ 繞路的形狀）",
                       "human Bearer token；被測的正是 allowWrite／toolScope／consentScope／dataScope／ttlMinutes／maxMessages／maxCost／workdir 這些授權欄位",
                       0.006, "USD 0.00（無模型呼叫）；k04.py 內嵌的 7 個探測合計 6 ms（claude 09:13:57.842–847Z、codex 09:11:36.430–436Z）；guard_probe.py 兩輪的 wall time 未計時 = unknown"),
})

cases.append({
    "id": "K-04-resume-unknown-id",
    "scenario": "用不存在／假造的 resumeProviderSessionId 建立 session——預期誠實拒絕（沒有紀錄就不知道上次授權了什麼）",
    "agentOrSurface": "HTTP /v1/agent-sessions（daemon 19080／19081，以及 19084／19085 的空字串與空白變體）",
    "evidenceLevel": "integration",
    "precondition": "daemon 內存有其他 session 記錄，但沒有任何 session 的 providerSessionId 等於待測的假造 id",
    "steps": [
        "產生一個隨機假造 id：\"thread-does-not-exist-<12 hex>\"",
        "POST /v1/agent-sessions {agentId:<claude-code|codex>, workdir:<同一個>, ttlMinutes:20, maxMessages:40, allowWrite:false, toolScope:[], consentScope:[], dataScope:[], resumeProviderSessionId:<假造 id>}",
        "記錄狀態碼與錯誤訊息；另外測 resumeProviderSessionId=\"\" 與 \"  \"（全空白）兩個邊界",
        "確認沒有 session 被建立、沒有子程序被 spawn",
    ],
    "expected": {
        "ui": "4xx 並說明「找不到上一次的授權紀錄」",
        "coreState": "不建立任何 session record",
        "effect": "不 spawn 任何 agent 子程序、不呼叫 provider",
    },
    "result": "correctly-blocked",
    "actual": ("兩個 agent 都回 403 policy_blocked：\"找不到上一次的授權紀錄，無法確認接續沒有放寬任何範圍；"
               "請重新建立一個明確授權的工作階段\"。空字串與全空白的 resumeProviderSessionId 也走同一條拒絕路徑"
               "（不是被當成「沒有帶 resume」而放行）。回應 body 沒有洩漏任何既有 session 的 id 或路徑。"
               "SSE 沒有 session.started，`ps` 沒有新的 claude/codex 子程序。"),
    "evidence": [
        R + "/claude/run.log:22 — probe/unknownId/create status 403，訊息「找不到上一次的授權紀錄…」",
        R + "/codex/run.log:26 — 同上（codex 側）",
        R + "/guard-claude/run.log:9-10 與 " + R + "/guard-codex/run.log:9-10 — empty-resume-id 與 resume-id-whitespace 皆 403",
        R + "/claude/raw.json JSON path $.probes[?(@.name=='unknownId')] → {\"status\":403,\"resp\":{\"error\":{\"code\":\"policy_blocked\",…}}}",
        "crates/interaction-runtime/src/agents.rs:453-468 — resumed_session_record 找不到且是 gateway agent ⇒ PolicyBlocked（靜態對照）",
    ],
    "observations": [
        "拒絕理由是「不確定就拒絕」而非「找不到 thread」——runtime 不去問 provider 這個 thread 存不存在，只看自己有沒有授權紀錄，符合 agents.rs:445-452 的註解。",
        "空字串 resumeProviderSessionId 被視為一次續開嘗試（而不是「沒帶」），所以也被擋——對呼叫端來說是「多送一個空欄位就整個被拒」，行為一致但值得寫進 API 文件。",
        "錯誤訊息不含任何既有 session 的 id／路徑，沒有藉錯誤訊息做 id 列舉的空間。",
    ],
    "productDefects": [],
    "limitations": "沒有測「假造 id 恰好等於同一台機器上另一個隔離 home 的真實 thread id」的情境（跨 home 不共用記錄，理論上仍會被擋，但未實測）。",
    "cleanup": "無 session 產生，無需清理；相關 daemon 皆已結束、埠已釋放。",
    "environment": env("claude-code + codex", "n/a（請求在 create 階段被拒，沒有 provider turn）",
                       "provider-local-log (not via gateway) — 本案未產生 provider turn",
                       "n/a", "resumed（帶假造 resumeProviderSessionId）", CLAUDE_WD + " 與 " + CODEX_WD,
                       "human Bearer token；resumeProviderSessionId 為隨機假造值／空字串／全空白", 0.002,
                       "USD 0.00（無模型呼叫）"),
})

commands = [
    {"name": "claude K-04 全序列（S1 兩輪 → close → 409 → S2 resume → 7 個 resume 探測）",
     "cmd": "cd " + R + " && unset INTERACT_AI_CLAUDE_BIN INTERACT_AI_CODEX_BIN && python3 k04.py --agent claude-code --port 19080 --out " + R + "/claude",
     "cwd": R, "start": "2026-09-07T17:10:53+0800", "end": "2026-09-07T17:13:59+0800", "seconds": 185.39, "exit": 0,
     "stdout": R + "/claude/run.log（24 行，最後一行 DONE {\"agent\":\"claude-code\",\"secret\":\"X55XY4F8\",\"pid1\":\"e63b51f8-0c9c-4431-accd-2d9dca4336ba\",\"wall\":185.39}）",
     "stderr": "（與 stdout 合流，無錯誤）"},
    {"name": "codex K-04 全序列（同上）",
     "cmd": "cd " + R + " && unset INTERACT_AI_CLAUDE_BIN INTERACT_AI_CODEX_BIN && python3 k04.py --agent codex --port 19081 --approve approve --out " + R + "/codex",
     "cwd": R, "start": "2026-09-07T17:11:15+0800", "end": "2026-09-07T17:11:37+0800", "seconds": 21.73, "exit": 0,
     "stdout": R + "/codex/run.log（28 行，最後一行 DONE {\"agent\":\"codex\",\"secret\":\"J8SZQ555\",\"pid1\":\"01a07b22-d9c6-7470-8812-03a491f9b5c1\",\"wall\":21.73}）",
     "stderr": "（與 stdout 合流，無錯誤）"},
    {"name": "claude 續接 argv 直接證據（同一個 home 重啟 daemon 再 resume 一次）",
     "cmd": "cd " + R + " && python3 argv_probe.py --agent claude-code --port 19082 --home \"$PWD/claude/home\" --workdir \"$PWD/claude/workdir\" --resume-id e63b51f8-0c9c-4431-accd-2d9dca4336ba --out \"$PWD/argv-claude\"",
     "cwd": R, "start": "2026-09-07T17:15:57+0800", "end": "2026-09-07T17:16:02+0800", "seconds": 5.61, "exit": 0,
     "stdout": R + "/argv-claude/run.log:5 — `claude … --safe-mode … --permission-mode plan --tools Read,Glob,Grep --resume e63b51f8-0c9c-4431-accd-2d9dca4336ba`",
     "stderr": "（與 stdout 合流，無錯誤）"},
    {"name": "codex 續接 argv／子程序快照（同一個 home 重啟 daemon 再 resume 一次）",
     "cmd": "cd " + R + " && python3 argv_probe.py --agent codex --port 19083 --home \"$PWD/codex/home\" --workdir \"$PWD/codex/workdir\" --resume-id 01a07b22-d9c6-7470-8812-03a491f9b5c1 --out \"$PWD/argv-codex\"",
     "cwd": R, "start": "2026-09-07T17:16:23+0800", "end": "2026-09-07T17:16:29+0800", "seconds": 6.64, "exit": 0,
     "stdout": R + "/argv-codex/run.log（48 行；:4 codex app-server、:9 chrome-devtools-mcp、:10 codegraph --mcp、:11 computer-use、:43 SkyComputerUseClient 帶 thread-id）",
     "stderr": "（與 stdout 合流，無錯誤）"},
    {"name": "claude 續開授權關卡 10 形狀探測",
     "cmd": "cd " + R + " && python3 guard_probe.py --agent claude-code --port 19084 --home \"$PWD/claude/home\" --workdir \"$PWD/claude/workdir\" --resume-id e63b51f8-0c9c-4431-accd-2d9dca4336ba --out \"$PWD/guard-claude\"",
     "cwd": R, "start": "unknown（前景執行未計時）", "end": "2026-09-07T17:18:41+0800", "seconds": -1, "exit": 0,
     "stdout": R + "/guard-claude/run.log（10 行 + DONE；7×403、3×200）",
     "stderr": "（與 stdout 合流，無錯誤）"},
    {"name": "codex 續開授權關卡 10 形狀探測",
     "cmd": "cd " + R + " && python3 guard_probe.py --agent codex --port 19085 --home \"$PWD/codex/home\" --workdir \"$PWD/codex/workdir\" --resume-id 01a07b22-d9c6-7470-8812-03a491f9b5c1 --out \"$PWD/guard-codex\"",
     "cwd": R, "start": "unknown（前景執行未計時）", "end": "2026-09-07T17:19:49+0800", "seconds": -1, "exit": 0,
     "stdout": R + "/guard-codex/run.log（10 行 + DONE；7×403、1×400、2×200）",
     "stderr": "（與 stdout 合流，無錯誤）"},
    {"name": "codex 任務全文是否進入 world-readable process 命令列",
     "cmd": "cd " + R + " && python3 psleak_probe.py \"$PWD/psleak\" 19086",
     "cwd": R, "start": "unknown（前景執行未計時）", "end": "2026-09-07T17:21:21+0800", "seconds": -1, "exit": 0,
     "stdout": R + "/psleak/run.log:3 — PS-HIT：SkyComputerUseClient argv 內含 \"client\":\"adaptive-interaction\" 與含唯一標記 PSLEAKHTYB7B72 的 input-messages",
     "stderr": "（與 stdout 合流，無錯誤）"},
    {"name": "收尾檢查：埠與子程序",
     "cmd": "lsof -nP -iTCP -sTCP:LISTEN | awk '$9 ~ /:1908[0-9]$/' ; ps -axo pid,ppid,etime,command | grep -E 'claude -p --input-format|codex app-server' | grep -v grep",
     "cwd": R, "start": "2026-09-07T17:28:16+0800", "end": "2026-09-07T17:28:16+0800", "seconds": 0.4, "exit": 1,
     "stdout": "兩個查詢皆無輸出（grep 無命中，故 exit=1）：19080-19089 全部釋放，無殘留 agent 子程序；自己 spawn 的 daemon（19080 pid 33157、19081 pid 35619、19082 pid 76108、19083 pid 79597，以及 19084/19085/19086 三支 guard/psleak daemon）皆已結束",
     "stderr": ""},
]

open_questions = [
    "codex app-server 是否有可確定性關閉 MCP／plugin 的旗標或 config 覆寫（例如 `-c mcp_servers={}` 或獨立 CODEX_HOME）？若有，codex 連接器應比照 claude 的 --strict-mcp-config 補上；若沒有，就必須把「codex session 會啟動使用者設定的 MCP server 與 computer-use helper」寫進 FEATURES/CHANGELOG 的已知限制，而不是留在文件宣稱的唯讀邊界裡。",
    "codex 在 untrusted 核可政策下，MCP tool call 是否真的會回到 runtime 變成 waiting-for-consent（codex.rs:470 有處理 \"mcpToolCall\"，但本輪任務沒觸發任何工具，未驗證）。這決定了上面那個缺陷是「只是多開程序」還是「可繞過 Policy Governor 的實際能力」。",
    "Claude Code 的 --resume 沿用同一個 provider session id 是 2.1.263 的行為；若未來版本改成 fork 出新 id，runtime 的 provider_session_id 就會和 resumeProviderSessionId 分岔，而目前的 record 沒有任何欄位記下「本 session 接續自哪一個 id」。要不要現在就補一個 resumed_from 欄位？",
    "續開授權比對完全依賴本機 session record；記錄被清掉（換 home／清 state）之後同一個 provider thread 就再也接不回去（會被「找不到上一次的授權紀錄」擋掉）。這是刻意的誠實設計，但需要確認產品面接受「換機器＝續接不了」。",
    "跨 agent 續接（拿 claude 的 thread id 去建 codex session，或反之）本輪因為兩個 agent 用不同隔離 home 而未涵蓋；同一個 home 內兩種 agent 記錄共存時，resumed_session_record 只比對 provider_session_id、不比對 agent_id（agents.rs:352-365），值得補一個案例。",
    "codex 完全不回報 USD 成本，且 token 累計在每次 resume 後歸零；跨 session 的累計用量目前沒有任何一層彙總。長跑的續接鏈要怎麼呈現「這條 thread 到目前為止花了多少」？",
]

notes = (
    "結論（誠實分類）：原生續接能力與授權保持在 v0.8.0 的兩個真連接器上都**實測成立**，六個 case 全部拿到"
    "「投影＋核心狀態＋實際效果」三件證據，沒有一個是靠 agent 自述或 exit 0 判定的。\n"
    "1) 同 session 多輪：claude 與 codex 都**不重新 spawn**子程序（pid 33355／35908 跨兩輪不變），第二輪都答對隨機暗號。\n"
    "2) 原生續接：claude 走 `--resume <id>`（argv 直接證據，argv-claude/run.log:5），codex 走 thread/resume；"
    "兩者的 provider 端記錄都**續寫同一個檔案**（claude 同一個 jsonl、codex 同一個 rollout），且兩個 provider 在這個版本"
    "**都沒有換 session id**。答案正確且已排除洩題——續接輪的 prompt 裡沒有暗號（只有 task-memory 摘要）。\n"
    "3) 授權保持：續接時授權旗標是**重新上鎖**而非繼承——claude argv 仍帶 --safe-mode／--permission-mode plan／"
    "--tools Read,Glob,Grep；codex 續接輪的 turn_context 仍是 read-only／untrusted／managed-restricted。\n"
    "4) 續開不得放寬：16 個放寬形狀（每個 agent）全部 4xx，訊息逐項指名；縮小形狀正確放行；workdir 的 ../ 繞路被"
    "canonicalize 擋下；假造／空白 resume id 一律拒絕。\n"
    "值得注意的三個負面發現：(a) codex 連接器沒有 claude 那樣的 MCP 封鎖，一個宣告唯讀的 session 一建立就啟動使用者"
    "設定的 chrome-devtools-mcp／codegraph／ChatGPT computer-use helper，而且任務全文會出現在 world-readable 的 "
    "process 命令列（已用唯一標記實測，不是推論）；(b) 續開授權的拒絕與接受都沒有稽核紀錄；(c) claude 續開 session 在 "
    "create 回應裡看不出自己接續了哪一個 thread。\n"
    "另一個對後續測試有影響的觀察：快輪的 REST state 投影幾乎不可觀測（claude 第二輪的 active 視窗約 11 ms），"
    "0.5 s 輪詢 GET /v1/agent-sessions/{id} 一次都沒抓到 active；判斷「任務有沒有被接走」必須看 SSE 或 mailbox。\n"
    "紀律聲明：全程沒有 cargo build／pnpm build，沒有修改 repo 內任何檔案；每個 daemon 都用自己的 INTERACT_AI_HOME"
    "（" + R + "/*/home）與指派埠段 19080-19086；只 kill 自己 spawn 的 daemon pid，沒有用過 pkill；"
    "~/.adaptive-interaction、~/.claude、~/.codex 全程只讀（provider log 當證據）。所有真 agent 都是真的"
    "（INTERACT_AI_CLAUDE_BIN／INTERACT_AI_CODEX_BIN 已 unset，無 fixture）。"
)

result = {"group": "K-04／B-08 原生續接能力及授權保持（真 Claude Code 與真 Codex）",
          "cases": cases, "commands": commands, "openQuestions": open_questions, "notes": notes}
path = os.path.join(R, "result-R8.json")
json.dump(result, open(path, "w"), ensure_ascii=False, indent=1)
print("wrote", path, os.path.getsize(path), "bytes;", len(cases), "cases,", len(commands), "commands")
