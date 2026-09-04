# `.claude/workflows/` — 對抗審查 workflow

這些是 Claude Code **Workflow** 腳本（`Workflow({ name: '<檔名去掉 .js>' })` 或
`/workflows` 啟動）。它們把「找缺陷 → 獨立懷疑者反駁 → 只留 confirmed」寫成確定性的
多 agent 編排；找到的缺陷由人（或主迴圈）決定修或記為已知限制。

| 腳本 | 範圍 | 輸出 |
|---|---|---|
| `adversarial-review-adaptive-interaction.js` | v0.3 治理核心（policy／receipt／async／API／recipe） | 回傳值 |
| `adversarial-review-v04.js` | v0.4 子系統（presentation／gateway／memory／knowledge／curator／前端） | 回傳值 |
| `adversarial-review-v05.js` | v0.5 三核心＋角色呈現協定，13 維度、blocker/high 雙視角驗證 | 回傳值＋`docs/reviews/adversarial/<runId>.{json,md}` |
| `adversarial-review-v06.js` | v0.6.0 Foundation：AIP／Session／身分／配對遷移／角色包／renderer 生命週期／一般模式／證據／重連／邊界／發布 12 維度（規格＝`docs/aip/README.md`） | 回傳值＋`docs/reviews/adversarial/<runId>.{json,md}` |

## 所需 runtime

- **Claude Code**（含 Workflow 工具；session 需開啟 workflow 編排，例如 ultracode 或明確要求「run a workflow」）。
  腳本本身沒有檔案系統／Node API，所有讀寫都由子 agent 執行。
- 在 **git checkout 內**的任一目錄啟動：preflight agent 以 `git rev-parse --show-toplevel` 解析 repo，
  不硬編任何絕對路徑；解析失敗即 throw。
- 子 agent 會用到 `git`、`cargo`（跑單一 crate 測試）、`pnpm`（`pnpm test`）、`rg`/`grep`/`sed`。
  它們**不會**啟動 daemon、不跑 Playwright（會撞 8787 埠）。
- v0.5 腳本要求規格檔存在於 repo：`docs/specs/adaptive-interaction-v05-core-experience-prompt.md`。
  缺檔即 fail-fast，不會憑記憶虛構規格。

## `adversarial-review-v05.js` 參數

```js
Workflow({ name: 'adversarial-review-v05' })
Workflow({ name: 'adversarial-review-v05', args: {
  seeds: [{ dimension, title, file, line, severity, claim, evidence }],  // 既有主張直接進 Verify
  skipDimensions: ['mobile-server'],   // 正在被並行修復的維度
  findModel: 'opus', verifyModel: 'sonnet',  // 避開單一模型速率上限
  outDir: 'docs/reviews/adversarial',  // repo 相對路徑
}})
```

輸出 JSON 每筆 finding 記錄：`id`（`F-<runId>-<dimension>-<seq>`）、`dimension`、`severity`
（verifier 修正後）與 `reportedSeverity`、`file:line`（含行號漂移修正）、`claim`、`evidence`、
`verdict`（confirmed／fixed-meanwhile／refuted／unverified）、每位 verifier 的視角與理由、
`fix`、`regressionTest`。Markdown 是同一份資料的人讀版。run ID＝`<HEAD 短 hash>-<UTC 時間>`。

## 不變量

- 腳本與子 agent **絕不** `git add`／`commit`／`push`／release／deploy；Persist 階段只寫檔案。
- Finder 不修改檔案；Verifier 不修改檔案。修復由主迴圈另行進行並附回歸測試。
- 每個維度最多取 10 筆 finding（超過會在 log 標明被丟掉的數量，不會假裝全覆蓋）。
