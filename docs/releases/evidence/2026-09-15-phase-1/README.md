# 階段 1 原始證據索引（2026-09-15）

這個目錄是**階段 1**（重要互動可追蹤與可靠性修正）的一手證據：真 Agent 追蹤驗收的 harness 輸出與 Rust workspace 回歸 log。
敘事文件是 `docs/releases/phase-1-progress.md`（§5／§8）與 `docs/acceptance-evidence.md` 階段 1 一節。

- **查核 checkpoint**：`c89a5c4d8646`（程式碼最終 commit `c6dd64e`＋docs commit）。
- **daemon binary**：`target/debug/interact-ai` sha256 `5bea07b88575817c…`（由 `c6dd64e` 建置；`real-agent/*/result.json` 的 `binarySha256` 逐字記錄）。
- **環境**：macOS 26.2 arm64；Claude Code 2.1.272（claude.ai 登入；provider 自報模型 `claude-fable-5-1`）；Codex CLI 0.154.0（ChatGPT 登入；自報 `gpt-6-astra`）。模型無法經 gateway 指定（K-01），`requestedModel` 一律 `not-specifiable-via-gateway`。
- **不歸檔**：`home/`（token、SQLite）、各情境 `work-*/` 工作目錄；`archive-evidence.py` 已掃描 api-token／agent-token／`Bearer`／`sk-`／`ghp_` 形狀，0 命中。

| 路徑 | 內容 | 層級 |
|---|---|---|
| `real-agent/claude-code/{run.txt,result.json,daemon.txt}` | `trace_e2e.py --agent claude-code --port 19140`：8 情境逐一 verdict、每情境的 `/v1/trace?sessionId=` kinds 與 `/v1/agent-sessions/{id}/activity` 摘要、daemon log | real-agent |
| `real-agent/codex/{run.txt,result.json,daemon.txt}` | 同上 `--agent codex --port 19141`（多一個 approval：人類 deny）；daemon log 內含 D16 擷取的 codex stderr 診斷（`agent.stderr` target） | real-agent |
| `workspace-test.txt` | `cargo test --workspace --no-fail-fast` 完整輸出（1335／1／1；flaky 說明見 progress §8.2） | unit／integration |
| `artifact-manifest.json` | 逐檔 sha256（original／archived） | — |

前兩輪（HEAD `2b1572c`／`a24153e`）的 harness 輸出未歸檔（其暴露的 D1／D4 已在本輪修掉並以最終輪重跑證明），只在 progress §5 敘述。
