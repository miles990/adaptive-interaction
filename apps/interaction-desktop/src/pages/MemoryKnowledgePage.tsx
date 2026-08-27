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
              const out = await api.memoryExport();
              setNotice(`已匯出 ${String((out as Record<string, unknown>).count)} 條（JSON 已在下方顯示，可自行複製保存）。`);
              console.info("memory export", out);
            }}
          >
            匯出全部
          </button>
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
      <p className="muted small">
        AI（含各 agent）只能提出<strong>候選</strong>；正式發布永遠需要你複審。
        主張必須附證據；類比與 AI 推測不能標成因果。
      </p>
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

function AssetsSection({ refreshKey }: { refreshKey: number }) {
  const [data, retry] = useAsync(() => api.assetsList(), [refreshKey]);
  const [impact, setImpact] = React.useState<Record<string, unknown> | null>(null);
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
    </Section>
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
