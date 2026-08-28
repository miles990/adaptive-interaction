// 記憶與知識（spec §16-1.F／§16）：分層記憶、知識候選複審、素材、
// Receipts、Context Bundle 預覽（「本次提供了哪些」）。
// 誠實原則貫穿：fact≠inference≠candidate；刪除永遠可能；影響先預覽。

import React from "react";
import { api } from "../api";
import { Badge, Section, StateView, useAsync } from "../ui";

type MkTab = "memory" | "knowledge" | "assets" | "receipts" | "bundle";

export const LAYER_LABEL: Record<string, string> = {
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
  candidate: { text: "等待確認", kind: "pending" },
};

/** 一般模式只有三區（spec §11）：關於我的記憶／小樞學會的知識／素材與來源。
 *  知識收據、Context Bundle 預覽與候選複審工具屬於技術細節，只在進階模式出現。 */
const SIMPLE_TABS: [MkTab, string][] = [
  ["memory", "關於我的記憶"],
  ["knowledge", "小樞學會的知識"],
  ["assets", "素材與來源"],
];

const ADVANCED_TABS: [MkTab, string][] = [
  ...SIMPLE_TABS,
  ["receipts", "知識收據"],
  ["bundle", "Context Bundle 預覽"],
];

export function MemoryKnowledgePage({
  refreshKey,
  advanced = false,
  onNavigate,
}: {
  refreshKey: number;
  advanced?: boolean;
  onNavigate?: (tab: string) => void;
}) {
  const [tab, setTab] = React.useState<MkTab>("memory");
  const tabs = advanced ? ADVANCED_TABS : SIMPLE_TABS;
  const active = tabs.some(([id]) => id === tab) ? tab : "memory";
  return (
    <div>
      <div className="hub-tabs" role="tablist" aria-label="記憶與知識分類">
        {tabs.map(([id, label]) => (
          <button
            key={id}
            role="tab"
            aria-selected={active === id}
            className={active === id ? "hub-tab active" : "hub-tab"}
            onClick={() => setTab(id)}
          >
            {label}
          </button>
        ))}
      </div>
      {active === "memory" && (
        <MemorySection refreshKey={refreshKey} advanced={advanced} onNavigate={onNavigate} />
      )}
      {active === "knowledge" && <KnowledgeSection refreshKey={refreshKey} advanced={advanced} />}
      {active === "assets" && <AssetsSection refreshKey={refreshKey} />}
      {active === "receipts" && <ReceiptsSection refreshKey={refreshKey} />}
      {active === "bundle" && <BundleSection advanced />}
    </div>
  );
}

function MemorySection({
  refreshKey,
  advanced,
  onNavigate,
}: {
  refreshKey: number;
  advanced: boolean;
  onNavigate?: (tab: string) => void;
}) {
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
      <Section title="關於我的記憶">
        <p className="muted small">
          每一條都標明：是事實還是推論、誰建立、保存多久。沒有你不能刪除的記憶——
          「永久」只代表「直到你刪除」。
        </p>
        <p className="muted small">
          小樞跟你玩耍、互動累積的角色記憶（喜歡的玩具、相處距離）在「小樞」頁，
          不會混進這裡，也不會因為一次行為就推論你的個性。
          {onNavigate && (
            <>
              {" "}
              <button onClick={() => onNavigate("companion")}>前往小樞</button>
            </>
          )}
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
      {!advanced && <BundleSection advanced={false} />}
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
  const [error, setError] = React.useState<string | null>(null);
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
      {status === "expired" && (
        <div className="muted small">
          已過保存期限，內容已停止使用；只能刪除（無法重新確認）。
        </div>
      )}
      {error && <div className="state-box state-error">{error}</div>}
      <div className="row wrap">
        <button
          className="danger"
          onClick={async () => {
            try {
              await api.memoryDelete(String(item.memoryId));
              setError(null);
              onChanged();
            } catch (e) {
              setError(`刪除失敗：${String(e)}。這筆記憶沒有變更。`);
            }
          }}
        >
          刪除
        </button>
        {/* 到期（expired）的記憶後端一律當作不存在：PATCH 會回 NotFound。
            介面不能留一顆按下去只會靜默失敗的「重新確認」按鈕——那等於
            謊稱還能救回來。只有「已過複查期」（stale）才真的可以延期。 */}
        {status === "stale" ? (
          <button
            onClick={async () => {
              // 重新確認：把複查期往後推 90 天（明確的人類動作）。
              const next = new Date(Date.now() + 90 * 24 * 3600 * 1000).toISOString();
              try {
                await api.memoryPatch(String(item.memoryId), {
                  retention: { ...(retention ?? {}), reviewAfter: next },
                });
                setError(null);
                onChanged();
              } catch (e) {
                setError(`重新確認失敗：${String(e)}。保存期限沒有變更。`);
              }
            }}
          >
            重新確認（再保留 90 天）
          </button>
        ) : null}
      </div>
    </div>
  );
}

/** spec §11 指定的人類文案。技術狀態字串只在進階模式的原始資料裡出現。 */
export const K_STATUS_LABEL: Record<string, { text: string; kind: "ok" | "warn" | "bad" | "pending" }> = {
  candidate: { text: "等待確認", kind: "pending" },
  active: { text: "已採用", kind: "ok" },
  stale: { text: "可能過期", kind: "warn" },
  disputed: { text: "有不同說法", kind: "warn" },
  superseded: { text: "已被新版取代", kind: "pending" },
  archived: { text: "已封存", kind: "pending" },
};

const K_STATUS_ORDER = [
  "candidate",
  "active",
  "stale",
  "disputed",
  "superseded",
  "archived",
] as const;

function KnowledgeSection({ refreshKey, advanced }: { refreshKey: number; advanced: boolean }) {
  const [status, setStatus] = React.useState("candidate");
  const [data, retry] = useAsync(
    () => api.knowledgeList(status || undefined, 100),
    [refreshKey, status]
  );
  const [notice, setNotice] = React.useState<string | null>(null);
  return (
    <Section title="小樞學會的知識">
      {advanced && (
        <KnowledgeUpdatePanel
          onCreated={() => {
            setStatus("candidate");
            retry();
          }}
        />
      )}
      <p className="muted small">
        {advanced
          ? "AI（含各 agent）只能提出候選；正式發布永遠需要你複審。主張必須附證據；類比與 AI 推測不能標成因果。"
          : "AI 只能提出說法，要你確認過才會被採用。每一條都要有來源；推測不會被寫成事實。"}
      </p>
      {advanced && <DomainPacksPanel refreshKey={refreshKey} />}
      <label className="field-label">
        狀態
        <select value={status} onChange={(e) => setStatus(e.target.value)}>
          {K_STATUS_ORDER.map((id) => (
            <option key={id} value={id}>
              {K_STATUS_LABEL[id].text}
            </option>
          ))}
          <option value="">全部</option>
        </select>
      </label>
      {notice && (
        <p className="muted small" role="status">
          {notice}
        </p>
      )}
      <CorrectionPanel advanced={advanced} onCreated={retry} />
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
                    {advanced ? `${String(n.nodeType)}・信心 ${Number(n.confidence ?? 0).toFixed(2)}・` : ""}
                    來源 {((n.evidence as unknown[] | undefined) ?? []).length} 筆
                  </div>
                  <details>
                    <summary className="muted small">內容與來源</summary>
                    {advanced ? (
                      <pre className="json-view small">{JSON.stringify(
                        { content: n.content, evidence: n.evidence, counterexamples: n.counterexamples, applicability: n.applicability },
                        null,
                        2
                      )}</pre>
                    ) : (
                      <>
                        <p className="small">{String(n.content ?? "（沒有內容）")}</p>
                        <ul className="plain-list small">
                          {((n.evidence as Record<string, unknown>[] | undefined) ?? []).map(
                            (ev, index) => (
                              <li key={index} className="muted">
                                {String(ev.url ?? ev.assetHash ?? ev.note ?? "未註明來源")}
                              </li>
                            )
                          )}
                          {((n.evidence as unknown[] | undefined) ?? []).length === 0 && (
                            <li className="muted">（沒有附上任何來源）</li>
                          )}
                        </ul>
                      </>
                    )}
                  </details>
                  {String(n.status) === "candidate" && (
                    <div className="row wrap">
                      <button
                        className="primary"
                        onClick={async () => {
                          try {
                            await api.knowledgeReview(String(n.nodeId), "approve");
                            setNotice("已採用。");
                            retry();
                          } catch (e) {
                            setNotice(`無法採用：${e}`);
                          }
                        }}
                      >
                        採用
                      </button>
                      <button
                        onClick={async () => {
                          await api.knowledgeReview(String(n.nodeId), "reject", "由控制中心拒絕");
                          setNotice("已拒絕並封存。");
                          retry();
                        }}
                      >
                        不採用
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
  void onCreated;
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
    </div>
  );
}

/** 糾正小樞的說法。一般模式與進階模式都要有 —— 但一般模式不用治理術語。 */
function CorrectionPanel({
  advanced,
  onCreated,
}: {
  advanced: boolean;
  onCreated: () => void;
}) {
  const [original, setOriginal] = React.useState("");
  const [correction, setCorrection] = React.useState("");
  const [scope, setScope] = React.useState("");
  const [notice, setNotice] = React.useState<string | null>(null);
  const [saving, setSaving] = React.useState(false);
  return (
    <details className="state-box">
      <summary>糾正小樞的記憶或說法</summary>
      <p className="muted small">
        {advanced
          ? "糾正先保存為可刪除的「關於我的記憶」，並建立待複審候選；不會直接變成普遍知識，也不會自動呼叫 Agent。"
          : "你的糾正會先存成可以隨時刪除的「關於我的記憶」，並排進等待你確認的清單；不會馬上變成小樞的通用說法，也不會自動叫 AI 去查。"}
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
            setNotice(
              advanced
                ? "已保存使用者糾正並建立知識候選；尚未發布，等待複審。"
                : "已保存你的糾正；還沒有被採用，等你確認。"
            );
            onCreated();
          } catch (e) {
            setNotice(`保存失敗：${e}`);
          } finally {
            setSaving(false);
          }
        }}
      >
        {saving ? "保存中…" : "保存糾正"}
      </button>
      {notice && (
        <p className="muted small" role="status">
          {notice}
        </p>
      )}
    </details>
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
            Complete 只代表本機處理器完成；OCR、轉錄與推論內容都還沒有被確認，不會自動變成小樞的知識。
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

function BundleSection({ advanced }: { advanced: boolean }) {
  const [task, setTask] = React.useState("");
  const [agent, setAgent] = React.useState("claude-code");
  const [bundle, setBundle] = React.useState<Record<string, unknown> | null>(null);
  return (
    <Section title={advanced ? "提供給 AI 的內容（Context Bundle 預覽）" : "本次會提供給 AI 的內容"}>
      <p className="muted small">
        {advanced
          ? "送任務給 agent 前可先看：這次實際會提供哪些記憶與知識、哪些被排除（過期需複查、敏感、對該 agent 不可見、未複審候選）。不傳完整對話或整個知識庫。"
          : "把任務交給 AI 之前，可以先看這次實際會提供哪些記憶，哪些被擋下來。小樞不會把整個記憶或對話都交出去。"}
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
          {advanced ? (
            <pre className="json-view small">{JSON.stringify(bundle, null, 2)}</pre>
          ) : (
            <BundleHumanSummary bundle={bundle} />
          )}
        </div>
      )}
    </Section>
  );
}

/** 一般模式的內容摘要：條目標題與被擋下來的原因，不倒原始 JSON。 */
function BundleHumanSummary({ bundle }: { bundle: Record<string, unknown> }) {
  const includes = (bundle.includes as Record<string, unknown>[] | undefined) ?? [];
  const excluded = (bundle.excluded as Record<string, number> | undefined) ?? {};
  const reasons: [string, string][] = [
    ["needsReview", "需要你重新確認"],
    ["sensitive", "標為敏感"],
    ["notVisibleToAgent", "這個 AI 看不到"],
  ];
  return (
    <div>
      <ul className="plain-list small">
        {includes.map((item) => (
          <li key={String(item.memoryId)}>
            {String(item.title)}
            <span className="muted">
              　{LAYER_LABEL[String(item.layer)] ?? String(item.layer)}
            </span>
          </li>
        ))}
        {includes.length === 0 && <li className="muted">這次不會提供任何記憶。</li>}
      </ul>
      <p className="muted small">
        擋下來的：
        {reasons
          .filter(([key]) => Number(excluded[key] ?? 0) > 0)
          .map(([key, label]) => `${label} ${Number(excluded[key])} 條`)
          .join("、") || "沒有"}
      </p>
    </div>
  );
}
