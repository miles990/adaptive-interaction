// 記憶與知識（spec §16-1.F／§16）：分層記憶、知識候選複審、素材、
// Receipts、Context Bundle 預覽（「本次提供了哪些」）。
// 誠實原則貫穿：fact≠inference≠candidate；刪除永遠可能；影響先預覽。

import React from "react";
import { api } from "../api";
import { Badge, Section, StateView, useAsync } from "../ui";

type MkTab = "memory" | "candidates" | "assets" | "receipts" | "bundle";

const LAYER_LABEL: Record<string, string> = {
  "persona-core": "角色核心",
  "character-memory": "角色經歷",
  "user-memory": "關於我的記憶",
  "world-knowledge": "世界觀",
  "domain-knowledge": "領域知識",
  "domain-know-how": "領域 Know-how",
  skill: "Skill",
  "task-memory": "任務記憶",
  "session-context": "對話暫存",
  "agent-handoff": "Agent 交接",
};

const KIND_LABEL: Record<string, { text: string; kind: "ok" | "warn" | "pending" }> = {
  fact: { text: "事實", kind: "ok" },
  inference: { text: "推論", kind: "warn" },
  preference: { text: "偏好", kind: "ok" },
  "know-how": { text: "Know-how", kind: "ok" },
  candidate: { text: "候選（待確認）", kind: "pending" },
};

export function MemoryKnowledgePage({ refreshKey }: { refreshKey: number }) {
  const [tab, setTab] = React.useState<MkTab>("memory");
  return (
    <div>
      <div className="hub-tabs" role="tablist" aria-label="記憶與知識分類">
        {(
          [
            ["memory", "記憶"],
            ["candidates", "知識與候選"],
            ["assets", "原始素材"],
            ["receipts", "知識收據"],
            ["bundle", "提供給 AI 的內容"],
          ] as [MkTab, string][]
        ).map(([id, label]) => (
          <button
            key={id}
            role="tab"
            aria-selected={tab === id}
            className={tab === id ? "hub-tab active" : "hub-tab"}
            onClick={() => setTab(id)}
          >
            {label}
          </button>
        ))}
      </div>
      {tab === "memory" && <MemorySection refreshKey={refreshKey} />}
      {tab === "candidates" && <KnowledgeSection refreshKey={refreshKey} />}
      {tab === "assets" && <AssetsSection refreshKey={refreshKey} />}
      {tab === "receipts" && <ReceiptsSection refreshKey={refreshKey} />}
      {tab === "bundle" && <BundleSection />}
    </div>
  );
}

function MemorySection({ refreshKey }: { refreshKey: number }) {
  const [layer, setLayer] = React.useState<string>("");
  const [data, retry] = useAsync(
    () => api.memoryList(layer || undefined, 200),
    [refreshKey, layer]
  );
  const [notice, setNotice] = React.useState<string | null>(null);
  // 匯出結果必須真的呈現在畫面上（不是只丟 devtools console）才可宣稱「已在下方顯示」。
  const [exported, setExported] = React.useState<Record<string, unknown> | null>(null);

  const restoreBackup = async (file: File | undefined) => {
    if (!file) return;
    if (file.size > 5 * 1024 * 1024) {
      setNotice("還原失敗：備份檔超過 5 MiB 安全上限。");
      return;
    }
    try {
      const parsed = JSON.parse(await file.text()) as Record<string, unknown>;
      const items = Array.isArray(parsed.items) ? parsed.items : null;
      if (!items || items.length > 1000) {
        throw new Error("格式不符或超過 1,000 條上限");
      }
      let restored = 0;
      for (const raw of items) {
        if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
          throw new Error(`第 ${restored + 1} 條不是物件`);
        }
        const source = raw as Record<string, unknown>;
        // 不信任備份中的身分、時間與狀態；每一筆都以目前人類明確匯入，
        // 重新經過 Runtime schema、Secret 與 retention 驗證，並取得新 ID。
        await api.memoryCreate({
          layer: source.layer,
          kind: source.kind,
          title: source.title,
          content: source.content,
          provenance: source.provenance,
          confidence: source.confidence,
          tags: source.tags,
          agentVisibility: source.agentVisibility,
          agentDenylist: source.agentDenylist,
          retention: source.retention,
        });
        restored += 1;
      }
      setNotice(`已還原 ${restored} 條；每一條都重新通過 Runtime 驗證並取得新 ID。`);
      retry();
    } catch (e) {
      setNotice(`還原失敗：${e}。已成功寫入的項目會保留，請依訊息檢查備份。`);
    }
  };

  return (
    <div>
      <Section title="小樞記住了什麼">
        <p className="muted small">
          每一條都標明：是事實還是推論、誰建立、保存多久。沒有你不能刪除的記憶——
          「永久」只代表「直到你刪除」。
        </p>
        <div className="row wrap">
          <label className="field-label">
            分層
            <select value={layer} onChange={(e) => setLayer(e.target.value)}>
              <option value="">全部</option>
              {Object.entries(LAYER_LABEL).map(([id, label]) => (
                <option key={id} value={id}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <button
            onClick={async () => {
              try {
                const out = (await api.memoryExport()) as Record<string, unknown>;
                setExported(out);
                setNotice(`已匯出 ${String(out.count)} 條（JSON 已在下方顯示，可自行複製保存）。`);
              } catch (e) {
                setExported(null);
                setNotice(`匯出失敗：${e}`);
              }
            }}
          >
            匯出全部
          </button>
          <label className="button-like">
            還原備份
            <input
              className="visually-hidden"
              type="file"
              accept="application/json,.json"
              aria-label="選擇記憶備份 JSON"
              onChange={(event) => void restoreBackup(event.target.files?.[0])}
            />
          </label>
          <button
            onClick={async () => {
              const out = await api.memoryClearSession();
              setNotice(`已清除 ${String((out as Record<string, unknown>).cleared)} 條對話暫存。`);
              retry();
            }}
          >
            清除短期記憶
          </button>
        </div>
        {notice && (
          <p className="muted small" role="status">
            {notice}
          </p>
        )}
        {exported && (
          <div className="state-box">
            <div className="row space-between">
              <strong>匯出結果</strong>
              <button onClick={() => setExported(null)}>關閉</button>
            </div>
            <pre className="json-view small">{JSON.stringify(exported, null, 2)}</pre>
          </div>
        )}
        <StateView state={data} empty="這個分層目前沒有記憶。">
          {(d) => (
            <div className="provider-list">
              {((d.items as Record<string, unknown>[] | undefined) ?? []).map((m) => (
                <MemoryCard key={String(m.memoryId)} item={m} onChanged={retry} />
              ))}
            </div>
          )}
        </StateView>
      </Section>
    </div>
  );
}

function MemoryCard({
  item,
  onChanged,
}: {
  item: Record<string, unknown>;
  onChanged: () => void;
}) {
  const kind = KIND_LABEL[String(item.kind)] ?? { text: String(item.kind), kind: "pending" as const };
  const status = String(item.status ?? "active");
  const retention = item.retention as Record<string, unknown> | undefined;
  const createdBy = item.createdBy as Record<string, unknown> | string | undefined;
  const creator =
    typeof createdBy === "object" && createdBy
      ? String((createdBy as Record<string, unknown>).kind) === "agent"
        ? `Agent（${String((createdBy as Record<string, unknown>).id)}）`
        : String((createdBy as Record<string, unknown>).kind) === "runtime"
          ? "系統"
          : "你"
      : "你";
  const retentionText = retention?.expiresAt
    ? `到期：${new Date(String(retention.expiresAt)).toLocaleDateString("zh-TW")}`
    : retention?.reviewAfter
      ? `複查：${new Date(String(retention.reviewAfter)).toLocaleDateString("zh-TW")}`
      : "保存到你刪除為止";
  return (
    <div className="provider-card">
      <div className="row space-between">
        <strong>{String(item.title)}</strong>
        <span className="row" style={{ gap: 6 }}>
          <Badge kind={kind.kind}>{kind.text}</Badge>
          {status === "stale" && <Badge kind="warn">已過複查期</Badge>}
          {status === "expired" && <Badge kind="bad">已到期</Badge>}
        </span>
      </div>
      <div className="muted small">
        {LAYER_LABEL[String(item.layer)] ?? String(item.layer)}・由{creator}建立・{retentionText}
      </div>
      <details>
        <summary className="muted small">內容</summary>
        <pre className="json-view small">{String(item.content)}</pre>
      </details>
      <div className="row wrap">
        <button
          className="danger"
          onClick={async () => {
            await api.memoryDelete(String(item.memoryId));
            onChanged();
          }}
        >
          刪除
        </button>
        {status !== "active" ? (
          <button
            onClick={async () => {
              // 重新確認：把複查期往後推 90 天（明確的人類動作）。
              const next = new Date(Date.now() + 90 * 24 * 3600 * 1000).toISOString();
              await api.memoryPatch(String(item.memoryId), {
                retention: { ...(retention ?? {}), reviewAfter: next },
              });
              onChanged();
            }}
          >
            重新確認（再保留 90 天）
          </button>
        ) : null}
      </div>
    </div>
  );
}

const K_STATUS_LABEL: Record<string, { text: string; kind: "ok" | "warn" | "bad" | "pending" }> = {
  candidate: { text: "候選（待複審）", kind: "pending" },
  active: { text: "已發布", kind: "ok" },
  stale: { text: "已過期需確認", kind: "warn" },
  disputed: { text: "有衝突", kind: "warn" },
  superseded: { text: "已被新版取代", kind: "pending" },
  archived: { text: "已封存", kind: "pending" },
};

function KnowledgeSection({ refreshKey }: { refreshKey: number }) {
  const [status, setStatus] = React.useState("candidate");
  const [data, retry] = useAsync(
    () => api.knowledgeList(status || undefined, 100),
    [refreshKey, status]
  );
  const [notice, setNotice] = React.useState<string | null>(null);
  return (
    <Section title="知識與候選">
      <KnowledgeUpdatePanel
        onCreated={() => {
          setStatus("candidate");
          retry();
        }}
      />
      <p className="muted small">
        AI（含各 agent）只能提出<strong>候選</strong>；正式發布永遠需要你複審。
        主張必須附證據；類比與 AI 推測不能標成因果。
      </p>
      <DomainPacksPanel refreshKey={refreshKey} />
      <label className="field-label">
        狀態
        <select value={status} onChange={(e) => setStatus(e.target.value)}>
          <option value="candidate">候選（待複審）</option>
          <option value="active">已發布</option>
          <option value="stale">已過期</option>
          <option value="disputed">有衝突</option>
          <option value="superseded">已被取代</option>
          <option value="archived">已封存</option>
          <option value="">全部</option>
        </select>
      </label>
      {notice && (
        <p className="muted small" role="status">
          {notice}
        </p>
      )}
      <StateView state={data} empty="這個狀態目前沒有知識項目。">
        {(d) => (
          <div className="provider-list">
            {((d.nodes as Record<string, unknown>[] | undefined) ?? []).map((n) => {
              const st = K_STATUS_LABEL[String(n.status)] ?? {
                text: String(n.status),
                kind: "pending" as const,
              };
              return (
                <div className="provider-card" key={String(n.nodeId)}>
                  <div className="row space-between">
                    <strong>{String(n.title)}</strong>
                    <Badge kind={st.kind}>{st.text}</Badge>
                  </div>
                  <div className="muted small">
                    {String(n.nodeType)}・信心 {Number(n.confidence ?? 0).toFixed(2)}・證據{" "}
                    {((n.evidence as unknown[] | undefined) ?? []).length} 項
                  </div>
                  <details>
                    <summary className="muted small">內容與證據</summary>
                    <pre className="json-view small">{JSON.stringify(
                      { content: n.content, evidence: n.evidence, counterexamples: n.counterexamples, applicability: n.applicability },
                      null,
                      2
                    )}</pre>
                  </details>
                  {String(n.status) === "candidate" && (
                    <div className="row wrap">
                      <button
                        className="primary"
                        onClick={async () => {
                          try {
                            await api.knowledgeReview(String(n.nodeId), "approve");
                            setNotice("已發布。");
                            retry();
                          } catch (e) {
                            setNotice(`無法發布：${e}`);
                          }
                        }}
                      >
                        核可發布
                      </button>
                      <button
                        onClick={async () => {
                          await api.knowledgeReview(String(n.nodeId), "reject", "由控制中心拒絕");
                          setNotice("已拒絕並封存。");
                          retry();
                        }}
                      >
                        拒絕
                      </button>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </StateView>
    </Section>
  );
}

function DomainPacksPanel({ refreshKey }: { refreshKey: number }) {
  const [data, retry] = useAsync(() => api.domainPacks(), [refreshKey]);
  const [notice, setNotice] = React.useState<string | null>(null);
  return (
    <div className="state-box">
      <div className="row space-between">
        <strong>內建 Domain Knowledge／Know-how／Skills</strong>
        <Badge kind="ok">本機版本化</Badge>
      </div>
      <p className="muted small">
        只有工作階段明確授權相同 Domain 時才會放進 Context Bundle。移除後不會在重啟時自行裝回；
        這些參考資料不能授權、改寫 Active Knowledge 或取代專家判斷。
      </p>
      {notice ? <p role="status" className="muted small">{notice}</p> : null}
      <StateView state={data} empty="沒有可用的 Domain Pack。">
        {(payload) => (
          <div className="provider-list">
            {((payload.packs as Record<string, unknown>[] | undefined) ?? []).map((entry) => {
              const pack = (entry.pack as Record<string, unknown> | undefined) ?? {};
              const installed = entry.installed === true;
              return (
                <div className="provider-card" key={String(pack.id)}>
                  <div className="row space-between">
                    <strong>{String(pack.displayName)}</strong>
                    <Badge kind={installed ? "ok" : "muted"}>{installed ? "已安裝" : "未安裝"}</Badge>
                  </div>
                  <div className="muted small">{String(pack.id)}・v{String(pack.version)}</div>
                  <details>
                    <summary className="muted small">概念、流程、品質與限制</summary>
                    <pre className="json-view small">{JSON.stringify({
                      concepts: pack.concepts,
                      principles: pack.principles,
                      workflow: pack.workflow,
                      heuristics: pack.heuristics,
                      failurePatterns: pack.failurePatterns,
                      counterexamples: pack.counterexamples,
                      qualityRubric: pack.qualityRubric,
                      verification: pack.verification,
                      sources: pack.sources,
                      applicability: pack.applicability,
                      limitations: pack.limitations,
                      supersedes: pack.supersedes,
                    }, null, 2)}</pre>
                  </details>
                  <button
                    className={installed ? "danger" : ""}
                    onClick={async () => {
                      try {
                        if (installed) await api.domainPackUninstall(String(pack.id));
                        else await api.domainPackInstall(String(pack.id));
                        setNotice(installed ? "已移除；重啟後仍保持移除。" : "已安裝。下一個明確授權此 Domain 的 Bundle 才可使用。");
                        retry();
                      } catch (error) {
                        setNotice(`Domain Pack 更新失敗：${error}`);
                      }
                    }}
                  >
                    {installed ? "移除" : "安裝"}
                  </button>
                </div>
              );
            })}
          </div>
        )}
      </StateView>
    </div>
  );
}

const UPDATE_TRIGGER_LABEL: Record<string, string> = {
  "user-added-asset": "加入了新素材",
  "source-changed": "已核准來源有變更",
  "repo-commit": "Repository 出現新 Commit",
  "task-artifact": "任務產生新 Artifact",
  "user-correction": "我糾正了小樞",
  "conflict-detected": "發現知識衝突",
  "review-overdue": "知識超過複查期限",
  "low-confidence-answer": "回答資料不足或信心低",
  "periodic-health-check": "定期低成本健檢",
};

function KnowledgeUpdatePanel({ onCreated }: { onCreated: () => void }) {
  const [trigger, setTrigger] = React.useState("user-added-asset");
  const [decision, setDecision] = React.useState<Record<string, unknown> | null>(null);
  const [original, setOriginal] = React.useState("");
  const [correction, setCorrection] = React.useState("");
  const [scope, setScope] = React.useState("");
  const [notice, setNotice] = React.useState<string | null>(null);
  const [saving, setSaving] = React.useState(false);
  return (
    <div className="state-box">
      <strong>知識何時更新、何時需要 AI</strong>
      <p className="muted small">
        兩個決策彼此獨立。這裡只顯示確定性決策，不會因為查看結果就啟動 Agent、外部研究或產生成本。
      </p>
      <div className="row wrap">
        <select value={trigger} onChange={(e) => setTrigger(e.target.value)}>
          {Object.entries(UPDATE_TRIGGER_LABEL).map(([value, label]) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
        <button
          onClick={async () => {
            setDecision((await api.knowledgeUpdateCheck(trigger)) as unknown as Record<string, unknown>);
          }}
        >
          查看決策
        </button>
      </div>
      {decision && (
        <div className="provider-card" aria-live="polite">
          <div className="row wrap">
            <Badge kind={decision.needsUpdate === true ? "warn" : "ok"}>
              {decision.needsUpdate === true ? "需要更新" : "不需更新"}
            </Badge>
            <Badge kind={decision.needsAi === true ? "pending" : "ok"}>
              {decision.needsAi === true ? "需要 AI 候選" : "不呼叫 AI"}
            </Badge>
            {decision.requiresUserAsk === true && <Badge kind="warn">執行前需要確認</Badge>}
          </div>
          <p className="muted small">{String(decision.reason ?? "")}</p>
          <details>
            <summary className="muted small">處理步驟</summary>
            <pre className="json-view small">{JSON.stringify(decision, null, 2)}</pre>
          </details>
        </div>
      )}
      <details>
        <summary>糾正小樞的記憶或說法</summary>
        <p className="muted small">
          糾正先保存為可刪除的「關於我的記憶」，並建立待複審候選；不會直接變成普遍知識，也不會自動呼叫 Agent。
        </p>
        <label className="field-label">
          原本哪裡不對（選填）
          <textarea value={original} onChange={(e) => setOriginal(e.target.value)} maxLength={2000} />
        </label>
        <label className="field-label">
          正確內容
          <textarea value={correction} onChange={(e) => setCorrection(e.target.value)} maxLength={2000} />
        </label>
        <label className="field-label">
          適用範圍（選填）
          <input value={scope} onChange={(e) => setScope(e.target.value)} maxLength={500} />
        </label>
        <button
          className="primary"
          disabled={saving || !correction.trim()}
          onClick={async () => {
            setSaving(true);
            setNotice(null);
            try {
              await api.knowledgeUserCorrection({
                originalAssumption: original.trim() || undefined,
                correction: correction.trim(),
                scope: scope.trim() || undefined,
              });
              setOriginal("");
              setCorrection("");
              setScope("");
              setNotice("已保存使用者糾正並建立知識候選；尚未發布，等待複審。");
              onCreated();
            } catch (e) {
              setNotice(`保存失敗：${e}`);
            } finally {
              setSaving(false);
            }
          }}
        >
          {saving ? "保存中…" : "保存糾正並建立候選"}
        </button>
        {notice && (
          <p className="muted small" role="status">
            {notice}
          </p>
        )}
      </details>
    </div>
  );
}

function AssetsSection({ refreshKey }: { refreshKey: number }) {
  const [data, retry] = useAsync(() => api.assetsList(), [refreshKey]);
  const [impact, setImpact] = React.useState<Record<string, unknown> | null>(null);
  const [derivatives, setDerivatives] = React.useState<Record<string, unknown> | null>(null);
  const [derivingHash, setDerivingHash] = React.useState<string | null>(null);
  const [sourcePreview, setSourcePreview] = React.useState<{
    payload: Record<string, unknown>;
    segment?: string;
  } | null>(null);
  const [text, setText] = React.useState("");
  return (
    <Section title="原始素材（內容定址、不可覆寫）">
      <p className="muted small">
        素材以內容雜湊保存：同樣的內容永遠是同一筆，AI 不能覆寫或刪除來源。
        刪除前會顯示影響（哪些知識與衍生資料會受影響）。
      </p>
      <div className="row wrap">
        <input
          value={text}
          placeholder="貼上一段文字素材…"
          onChange={(e) => setText(e.target.value)}
        />
        <button
          disabled={!text.trim()}
          onClick={async () => {
            await api.assetImport({ content: text });
            setText("");
            retry();
          }}
        >
          加入素材
        </button>
      </div>
      <StateView state={data} empty="還沒有素材。">
        {(d) => (
          <div className="provider-list">
            {((d.assets as Record<string, unknown>[] | undefined) ?? []).map((a) => (
              <div className="provider-card" key={String(a.hash)}>
                <div className="row space-between">
                  <strong>{String(a.originalName ?? `${String(a.hash).slice(0, 12)}…`)}</strong>
                  <Badge kind="ok">{String(a.mediaType)}</Badge>
                </div>
                <div className="muted small">
                  {Number(a.sizeBytes ?? 0)} bytes・{String(a.source)}・hash {String(a.hash).slice(0, 16)}…
                </div>
                <div className="row wrap">
                  <button
                    onClick={async () => {
                      setSourcePreview({ payload: await api.assetPreview(String(a.hash)) });
                    }}
                  >
                    開啟來源
                  </button>
                  <button
                    disabled={derivingHash === String(a.hash)}
                    onClick={async () => {
                      const hash = String(a.hash);
                      setDerivingHash(hash);
                      try {
                        setDerivatives(await api.assetDerive(hash));
                        retry();
                      } finally {
                        setDerivingHash(null);
                      }
                    }}
                  >
                    {derivingHash === String(a.hash) ? "解析中…" : "本機解析素材"}
                  </button>
                  <button
                    onClick={async () => {
                      setDerivatives(await api.assetDerivatives(String(a.hash)));
                    }}
                  >
                    查看衍生資料
                  </button>
                  <button
                    onClick={async () => {
                      setImpact(await api.assetImpact(String(a.hash)));
                    }}
                  >
                    刪除影響預覽
                  </button>
                  <button
                    className="danger"
                    onClick={async () => {
                      await api.assetDelete(String(a.hash));
                      setImpact(null);
                      retry();
                    }}
                  >
                    刪除
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </StateView>
      {impact && (
        <div className="state-box">
          <strong>刪除影響</strong>
          <pre className="json-view small">{JSON.stringify(impact, null, 2)}</pre>
        </div>
      )}
      {derivatives && (
        <div className="state-box" data-testid="asset-derivative-viewer">
          <div className="row space-between">
            <strong>衍生資料與精確來源</strong>
            <button onClick={() => setDerivatives(null)}>關閉</button>
          </div>
          <p className="muted small">
            Complete 只代表本機處理器完成；OCR、轉錄與推論內容仍是未受信任候選，不會自動發布為知識。
          </p>
          <div className="provider-list">
            {((derivatives.derivatives as Record<string, unknown>[] | undefined) ?? []).map((item) => {
              const source = (item.source as Record<string, unknown> | undefined) ?? {};
              return (
                <div className="provider-card" key={String(item.derivativeId)}>
                  <div className="row space-between">
                    <strong>{String(item.kind)}</strong>
                    <Badge kind={item.status === "complete" ? "ok" : "pending"}>
                      {String(item.status)}
                    </Badge>
                  </div>
                  <div className="muted small">
                    {String(item.processor)} {String(item.processorVersion)}・
                    {String(source.segment ?? "無區域／時碼")}
                  </div>
                  <div className="muted small">{String(item.detail)}</div>
                  {item.outputHash ? (
                    <>
                      <code className="small">output {String(item.outputHash)}</code>
                      <button
                        onClick={async () => {
                          setSourcePreview({
                            payload: await api.assetPreview(String(item.outputHash)),
                            segment: String(source.segment ?? ""),
                          });
                        }}
                      >
                        預覽衍生內容
                      </button>
                    </>
                  ) : null}
                </div>
              );
            })}
          </div>
        </div>
      )}
      {sourcePreview && (
        <SourceMediaViewer
          payload={sourcePreview.payload}
          segment={sourcePreview.segment}
          onClose={() => setSourcePreview(null)}
        />
      )}
    </Section>
  );
}

export interface ParsedSourceSegment {
  region?: { x: number; y: number; width: number; height: number };
  startSeconds?: number;
  endSeconds?: number;
}

/** Parse only the documented deterministic segment grammar. Invalid,
 * full/unknown, negative, and reversed ranges stay unspecified; the viewer
 * never invents a location. */
export function parseSourceSegment(segment?: string): ParsedSourceSegment {
  if (!segment) return {};
  const parsed: ParsedSourceSegment = {};
  const region = segment.match(/(?:^|;)region=([0-9.]+),([0-9.]+),([0-9.]+),([0-9.]+)(?:;|$)/);
  if (region) {
    const [x, y, width, height] = region.slice(1).map(Number);
    if ([x, y, width, height].every(Number.isFinite) && x >= 0 && y >= 0 && width > 0 && height > 0) {
      parsed.region = { x, y, width, height };
    }
  }
  const time = segment.match(/(?:^|;)t=([0-9.]+)-([0-9.]+)(?:;|$)/);
  if (time) {
    const startSeconds = Number(time[1]);
    const endSeconds = Number(time[2]);
    if (Number.isFinite(startSeconds) && Number.isFinite(endSeconds) && startSeconds >= 0 && endSeconds > startSeconds) {
      parsed.startSeconds = startSeconds;
      parsed.endSeconds = endSeconds;
    }
  }
  return parsed;
}

export function SourceMediaViewer({
  payload,
  segment,
  onClose,
}: {
  payload: Record<string, unknown>;
  segment?: string;
  onClose: () => void;
}) {
  const mediaType = String(payload.mediaType ?? "other");
  const mime = String(payload.mime ?? "application/octet-stream");
  const encoded = String(payload.dataBase64 ?? "");
  const dataUrl = `data:${mime};base64,${encoded}`;
  const parsedSegment = parseSourceSegment(segment);
  const [imageSize, setImageSize] = React.useState<{ width: number; height: number } | null>(null);
  const seekToSegment = (element: HTMLAudioElement | HTMLVideoElement) => {
    if (parsedSegment.startSeconds !== undefined) element.currentTime = parsedSegment.startSeconds;
  };
  const stopAtSegmentEnd = (element: HTMLAudioElement | HTMLVideoElement) => {
    if (parsedSegment.endSeconds !== undefined && element.currentTime >= parsedSegment.endSeconds) {
      element.pause();
      element.currentTime = parsedSegment.endSeconds;
    }
  };
  let text = "";
  if (["text", "code", "data"].includes(mediaType)) {
    try {
      const binary = atob(encoded);
      const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
      text = new TextDecoder().decode(bytes);
    } catch {
      text = "預覽解碼失敗；原始素材沒有被修改。";
    }
  }
  return (
    <div className="state-box source-media-viewer" data-testid="source-media-viewer">
      <div className="row space-between">
        <strong>來源檢視器</strong>
        <button onClick={onClose}>關閉</button>
      </div>
      <div className="muted small">
        hash {String(payload.hash).slice(0, 20)}…・{Number(payload.sizeBytes ?? 0)} bytes
        {segment ? `・精確引用 ${segment}` : ""}
      </div>
      {mediaType === "image" ? (
        <figure>
          <div className="source-image-frame">
            <img
              src={dataUrl}
              alt={`內容定址來源 ${String(payload.hash).slice(0, 12)}`}
              onLoad={(event) =>
                setImageSize({
                  width: event.currentTarget.naturalWidth,
                  height: event.currentTarget.naturalHeight,
                })
              }
            />
            {parsedSegment.region && imageSize ? (
              <span
                className="source-region-overlay"
                role="img"
                aria-label={`引用區域 ${segment}`}
                style={{
                  left: `${(parsedSegment.region.x / imageSize.width) * 100}%`,
                  top: `${(parsedSegment.region.y / imageSize.height) * 100}%`,
                  width: `${(parsedSegment.region.width / imageSize.width) * 100}%`,
                  height: `${(parsedSegment.region.height / imageSize.height) * 100}%`,
                }}
              />
            ) : null}
          </div>
          <figcaption className="muted small">{segment || "完整圖片"}</figcaption>
        </figure>
      ) : null}
      {mediaType === "audio" ? (
        <audio
          controls
          preload="metadata"
          src={dataUrl}
          onLoadedMetadata={(event) => seekToSegment(event.currentTarget)}
          onTimeUpdate={(event) => stopAtSegmentEnd(event.currentTarget)}
        />
      ) : null}
      {mediaType === "video" ? (
        <video
          controls
          preload="metadata"
          src={dataUrl}
          onLoadedMetadata={(event) => seekToSegment(event.currentTarget)}
          onTimeUpdate={(event) => stopAtSegmentEnd(event.currentTarget)}
        />
      ) : null}
      {mediaType === "pdf" ? (
        <object data={dataUrl} type="application/pdf" aria-label="PDF 來源預覽">
          此環境無法內嵌 PDF；可使用匯出功能查看原始素材。
        </object>
      ) : null}
      {text ? <pre className="json-view small">{text}</pre> : null}
      <p className="muted small">{String(payload.note ?? "")}</p>
    </div>
  );
}

function ReceiptsSection({ refreshKey }: { refreshKey: number }) {
  const [data] = useAsync(() => api.knowledgeReceipts(), [refreshKey]);
  return (
    <Section title="知識收據（每次知識變化的機器可讀紀錄）">
      <StateView state={data} empty="還沒有知識變化紀錄。">
        {(d) => (
          <div className="provider-list">
            {((d.receipts as Record<string, unknown>[] | undefined) ?? []).slice(0, 30).map((r) => {
              const v = r.verification as Record<string, unknown> | undefined;
              const p = r.published as Record<string, unknown> | undefined;
              return (
                <div className="provider-card" key={String(r.updateId)}>
                  <div className="row space-between">
                    <strong>{String(r.triggeredBy)}</strong>
                    <span className="row" style={{ gap: 6 }}>
                      {v?.humanReviewed === true ? (
                        <Badge kind="ok">已人工複審</Badge>
                      ) : (
                        <Badge kind="pending">未經人工複審</Badge>
                      )}
                      {p?.claims === true ? (
                        <Badge kind="ok">已發布</Badge>
                      ) : (
                        <Badge kind="pending">候選／未發布</Badge>
                      )}
                    </span>
                  </div>
                  <div className="muted small">
                    {new Date(String(r.createdAt)).toLocaleString("zh-TW")}・衝突檢查：
                    {String(v?.conflictCheck ?? "unknown")}
                  </div>
                  <details>
                    <summary className="muted small">變更明細</summary>
                    <pre className="json-view small">{JSON.stringify(r.changes, null, 2)}</pre>
                  </details>
                </div>
              );
            })}
          </div>
        )}
      </StateView>
    </Section>
  );
}

function BundleSection() {
  const [task, setTask] = React.useState("");
  const [agent, setAgent] = React.useState("claude-code");
  const [bundle, setBundle] = React.useState<Record<string, unknown> | null>(null);
  return (
    <Section title="提供給 AI 的內容（Context Bundle 預覽）">
      <p className="muted small">
        送任務給 agent 前可先看：這次<strong>實際會提供哪些</strong>記憶與知識、哪些被排除
        （過期需複查、敏感、對該 agent 不可見、未複審候選）。不傳完整對話或整個知識庫。
      </p>
      <div className="row wrap">
        <input value={task} placeholder="任務描述…" onChange={(e) => setTask(e.target.value)} />
        <select value={agent} onChange={(e) => setAgent(e.target.value)}>
          <option value="claude-code">Claude Code</option>
          <option value="codex">Codex</option>
        </select>
        <button
          disabled={!task.trim()}
          onClick={async () => {
            setBundle(await api.memoryBundle(task, agent, []));
          }}
        >
          預覽
        </button>
      </div>
      {bundle && (
        <div className="state-box">
          <strong>
            會提供 {((bundle.includes as unknown[] | undefined) ?? []).length} 條
          </strong>
          <pre className="json-view small">{JSON.stringify(bundle, null, 2)}</pre>
        </div>
      )}
    </Section>
  );
}
