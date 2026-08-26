// 自動互動（Recipe）：句子式編輯器、自然語言摘要、情境模擬、影響預覽。
// 一般模式完全不需要 YAML；儲存時經由後端單一模型轉換，未知欄位不會遺失。

import React from "react";
import { api, ConvertResult, HumanCard, ScenarioReport } from "../api";
import { useAppState } from "../appstate";
import { Icon } from "../icons";
import { Badge, Section, Toggle } from "../ui";
import { ConfirmButton, Dialog } from "../components/Dialog";

type RecipeJson = Record<string, any>;

export function AutomationsPage({
  refreshKey,
  advanced,
}: {
  refreshKey: number;
  advanced: boolean;
}) {
  const [list, setList] = React.useState<{ recipe: RecipeJson; state: RecipeJson }[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [editing, setEditing] = React.useState<RecipeJson | null>(null);
  const [simulating, setSimulating] = React.useState<string | null>(null);
  const [preview, setPreview] = React.useState<RecipeJson | null>(null);

  const reload = React.useCallback(() => {
    api
      .recipesList()
      .then((r) => {
        setList(r as { recipe: RecipeJson; state: RecipeJson }[]);
        setError(null);
      })
      .catch((e) => setError(String(e)));
  }, []);
  React.useEffect(reload, [refreshKey]);

  if (error)
    return (
      <div className="state-box state-error">
        無法載入自動互動：{error} <button onClick={reload}>重試</button>
      </div>
    );
  if (!list) return <div className="state-box">載入中…</div>;

  return (
    <div>
      <p className="page-intro">
        自動互動是「當…就…」的規則：系統感知到事件後，依安全規則自動回應。啟用前會先顯示影響預覽。
      </p>
      <div className="row wrap" style={{ marginBottom: 12 }}>
        <button onClick={() => setEditing(newRecipeTemplate())}>
          <Icon name="plus" size={14} /> 建立自動互動
        </button>
      </div>
      {list.length === 0 ? (
        <div className="state-box">
          還沒有自動互動。點「建立自動互動」開始，或到設定頁重新執行首次設定精靈套用範本。
        </div>
      ) : (
        list.map(({ recipe }) => (
          <RecipeRow
            key={String(recipe.id)}
            recipe={recipe}
            advanced={advanced}
            onEdit={() => setEditing(recipe)}
            onSimulate={() => setSimulating(String(recipe.id))}
            onToggle={async (enabled) => {
              if (enabled) {
                setPreview(recipe); // 啟用前先看影響
              } else {
                await api.recipeSetEnabled(String(recipe.id), false);
                reload();
              }
            }}
            onDelete={async () => {
              await api.recipeDelete(String(recipe.id));
              reload();
            }}
          />
        ))
      )}

      {editing && (
        <RecipeEditor
          initial={editing}
          advanced={advanced}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            reload();
          }}
        />
      )}
      {simulating && (
        <SimulateDialog recipeId={simulating} onClose={() => setSimulating(null)} />
      )}
      {preview && (
        <ImpactPreviewDialog
          recipe={preview}
          onCancel={() => setPreview(null)}
          onConfirm={async () => {
            await api.recipeSetEnabled(String(preview.id), true);
            setPreview(null);
            reload();
          }}
        />
      )}
    </div>
  );
}

function RecipeRow({
  recipe,
  advanced,
  onEdit,
  onSimulate,
  onToggle,
  onDelete,
}: {
  recipe: RecipeJson;
  advanced: boolean;
  onEdit: () => void;
  onSimulate: () => void;
  onToggle: (enabled: boolean) => void;
  onDelete: () => void;
}) {
  const [summary, setSummary] = React.useState<string>("");
  React.useEffect(() => {
    api
      .recipeSummary(String(recipe.id))
      .then((r) => setSummary(r.summary))
      .catch(() => setSummary(""));
  }, [recipe]);
  const enabled = recipe.enabled !== false;
  const aiMode = recipe.ai?.mode ?? "never";
  return (
    <Section
      title={String(recipe.name ?? recipe.id)}
      actions={
        <div className="row wrap">
          {aiMode !== "never" && <Badge kind="info">會用到 AI</Badge>}
          <Badge kind={enabled ? "ok" : "muted"}>{enabled ? "啟用中" : "已停用"}</Badge>
        </div>
      }
    >
      {summary && <p className="recipe-summary">{summary}</p>}
      <div className="row wrap">
        <Toggle checked={enabled} onChange={onToggle} label={enabled ? "啟用中" : "已停用"} />
        <button onClick={onSimulate}>模擬</button>
        <button onClick={onEdit}>編輯</button>
        <ConfirmButton label="刪除" confirmLabel="確定刪除？" onConfirm={onDelete} />
        {advanced && <span className="muted small"><code>{String(recipe.id)}</code></span>}
      </div>
    </Section>
  );
}

// ---------------------------------------------------------------------------
// 句子式編輯器
// ---------------------------------------------------------------------------

function newRecipeTemplate(): RecipeJson {
  return {
    id: `recipe-${Date.now().toString(36)}`,
    name: "新的自動互動",
    enabled: false,
    trigger: { mode: "single", steps: [{ receptor: "task.lifecycle", condition: { event: "task.completed" } }] },
    decision: { objective: "respond-helpfully", allowNoAction: true },
    intent: "success",
    message: { mode: "adaptive", templates: [], allowSilence: true },
    actuation: { mode: "adaptive", candidates: ["conversation"], minChannels: 0, maxChannels: 1 },
    verification: { strategy: "best-effort" },
    limits: { cooldown: "10m" },
  };
}

/** 判斷是否含本編輯器無法完整呈現的進階結構（仍會完整保留）。 */
function hasAdvancedParts(r: RecipeJson): boolean {
  const steps = r.trigger?.steps ?? [];
  if (steps.length > 1) return true;
  if (r.trigger?.mode && r.trigger.mode !== "single") return true;
  const cond = steps[0]?.condition;
  if (cond && typeof cond === "object" && Object.keys(cond).length > 1) return true;
  if (Object.keys(r).some((k) => !KNOWN_TOP_KEYS.includes(k))) return true;
  return false;
}

const KNOWN_TOP_KEYS = [
  "id", "name", "description", "enabled", "trigger", "context", "decision", "intent",
  "message", "actuation", "verification", "limits", "consent", "metadata", "ai", "schemaVersion",
];

function RecipeEditor({
  initial,
  advanced,
  onClose,
  onSaved,
}: {
  initial: RecipeJson;
  advanced: boolean;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { human } = useAppState();
  // 深拷貝：編輯只動已知欄位，未知欄位原樣保留在物件裡。
  const [r, setR] = React.useState<RecipeJson>(() => JSON.parse(JSON.stringify(initial)));
  const [issues, setIssues] = React.useState<{ field: string; message: string }[]>([]);
  const [saving, setSaving] = React.useState(false);
  const [yamlView, setYamlView] = React.useState<string | null>(null);
  const advancedParts = hasAdvancedParts(initial);

  const receptors = human?.receptors ?? [];
  const actuators = human?.actuators ?? [];
  const step = r.trigger?.steps?.[0] ?? {};
  const condEntries = Object.entries(step.condition ?? {});
  const [condKey, condValue] = condEntries[0] ?? ["", ""];

  function set(path: string[], value: unknown) {
    setR((prev) => {
      const next = JSON.parse(JSON.stringify(prev));
      let node = next;
      for (const key of path.slice(0, -1)) {
        if (typeof node[key] !== "object" || node[key] === null) node[key] = {};
        node = node[key];
      }
      node[path[path.length - 1]] = value;
      return next;
    });
  }

  async function save() {
    setSaving(true);
    setIssues([]);
    try {
      // 經同一個後端驗證器＋模型轉 YAML；未知欄位由 flatten 保留。
      const result: ConvertResult = await api.recipeConvert(JSON.stringify(r), "yaml");
      if (!result.valid) {
        setIssues(result.issues ?? [{ field: "$", message: "驗證失敗" }]);
        return;
      }
      await api.recipeUpsert(result.text!);
      onSaved();
    } catch (e) {
      setIssues([{ field: "$", message: String(e) }]);
    } finally {
      setSaving(false);
    }
  }

  const issueFor = (prefix: string) =>
    issues.filter((i) => i.field === prefix || i.field.startsWith(prefix + "."));

  const candidates: string[] = r.actuation?.candidates ?? [];

  return (
    <Dialog title={String(r.name ?? "編輯自動互動")} onClose={onClose}>
      {advancedParts && (
        <p className="notice-box">
          這個配方包含進階設定（多重觸發、複雜條件或自訂欄位）。此處只編輯基本欄位，
          其餘內容會原封不動保留。
        </p>
      )}
      <div className="sentence-form">
        <label className="sentence-row">
          <span className="sentence-word">名稱</span>
          <input
            value={String(r.name ?? "")}
            onChange={(e) => set(["name"], e.target.value)}
            aria-label="配方名稱"
          />
        </label>
        <FieldIssues issues={issueFor("name")} />

        <label className="sentence-row">
          <span className="sentence-word">當</span>
          <select
            value={String(step.receptor ?? "")}
            onChange={(e) => set(["trigger", "steps"], [{ ...step, receptor: e.target.value }])}
            aria-label="觸發的感知來源"
            disabled={advancedParts}
          >
            {receptors.map((c) => (
              <option key={c.id} value={c.id}>
                {c.displayName}
              </option>
            ))}
            {!receptors.some((c) => c.id === step.receptor) && step.receptor && (
              <option value={step.receptor}>{step.receptor}</option>
            )}
          </select>
          <span className="sentence-word">而且</span>
          <input
            style={{ width: 90 }}
            placeholder="欄位"
            value={String(condKey)}
            aria-label="條件欄位"
            disabled={advancedParts}
            onChange={(e) => {
              const cond = e.target.value ? { [e.target.value]: condValue } : undefined;
              set(["trigger", "steps"], [{ ...step, condition: cond }]);
            }}
          />
          <span className="sentence-word">=</span>
          <input
            style={{ width: 120 }}
            placeholder="值"
            value={String(condValue ?? "")}
            aria-label="條件值"
            disabled={advancedParts || !condKey}
            onChange={(e) =>
              set(["trigger", "steps"], [{ ...step, condition: { [condKey]: e.target.value } }])
            }
          />
        </label>
        <FieldIssues issues={issueFor("trigger")} />

        <div className="sentence-row">
          <span className="sentence-word">需要時</span>
          <select
            value={String(r.ai?.mode ?? "never")}
            aria-label="AI 介入方式"
            onChange={(e) => {
              if (e.target.value === "never") {
                setR((prev) => {
                  const next = { ...prev };
                  delete next.ai;
                  return next;
                });
              } else {
                set(["ai"], { ...(r.ai ?? {}), mode: e.target.value });
              }
            }}
          >
            <option value="never">不使用 AI（完全由本機規則處理）</option>
            <option value="when-uncertain">只有無法判斷時請 AI 協助</option>
            <option value="generate-text">AI 只生成文字內容</option>
          </select>
          {r.ai?.mode === "when-uncertain" && (
            <select
              value={String(r.ai?.onUnavailable ?? "fallback")}
              aria-label="AI 不可用時"
              onChange={(e) => set(["ai", "onUnavailable"], e.target.value)}
            >
              <option value="fallback">AI 沒回應 → 用本機規則處理</option>
              <option value="no-action">AI 沒回應 → 這次不介入</option>
            </select>
          )}
        </div>
        <FieldIssues issues={issueFor("ai")} />

        <div className="sentence-row">
          <span className="sentence-word">就用</span>
          <div className="candidate-picker" role="group" aria-label="回應方式">
            {actuators.map((a) => (
              <label key={a.id} className={candidates.includes(a.id) ? "candidate on" : "candidate"}>
                <input
                  type="checkbox"
                  checked={candidates.includes(a.id)}
                  onChange={(e) => {
                    const next = e.target.checked
                      ? [...candidates, a.id]
                      : candidates.filter((c) => c !== a.id);
                    set(["actuation", "candidates"], next);
                  }}
                />
                <Icon name={a.icon} size={14} /> {a.displayName}
              </label>
            ))}
          </div>
          <select
            value={String(r.actuation?.mode ?? "adaptive")}
            aria-label="使用方式"
            onChange={(e) => set(["actuation", "mode"], e.target.value)}
          >
            <option value="adaptive">挑最不打擾的一種</option>
            <option value="single">只用第一種</option>
            <option value="fallback">優先第一種，失敗換下一種</option>
            <option value="parallel">全部同時</option>
            <option value="sequence">依序使用</option>
          </select>
        </div>
        <FieldIssues issues={issueFor("actuation")} />

        <div className="sentence-row">
          <span className="sentence-word">說什麼</span>
          <select
            value={String(r.message?.mode ?? "adaptive")}
            aria-label="文字模式"
            onChange={(e) => set(["message", "mode"], e.target.value)}
          >
            <option value="adaptive">從候選文字中自動挑選</option>
            <option value="fixed">固定使用第一句</option>
            <option value="random">隨機挑一句</option>
            <option value="none">不顯示文字</option>
          </select>
          <label className="toggle">
            <input
              type="checkbox"
              checked={Boolean(r.message?.allowSilence ?? true)}
              onChange={(e) => set(["message", "allowSilence"], e.target.checked)}
            />
            <span>允許保持安靜</span>
          </label>
        </div>
        {r.message?.mode !== "none" && (
          <TemplateList
            templates={(r.message?.templates as string[]) ?? []}
            onChange={(t) => set(["message", "templates"], t)}
          />
        )}

        <div className="sentence-row">
          <span className="sentence-word">限制</span>
          <label>
            冷卻
            <input
              style={{ width: 70 }}
              value={String(r.limits?.cooldown ?? "")}
              placeholder="10m"
              aria-label="冷卻時間"
              onChange={(e) => set(["limits", "cooldown"], e.target.value || undefined)}
            />
          </label>
          <label>
            每小時最多
            <input
              type="number"
              min={0}
              style={{ width: 60 }}
              value={r.limits?.maxPerHour ?? ""}
              aria-label="每小時上限"
              onChange={(e) =>
                set(["limits", "maxPerHour"], e.target.value ? Number(e.target.value) : undefined)
              }
            />
          </label>
          <label className="toggle">
            <input
              type="checkbox"
              checked={Boolean(r.decision?.allowNoAction ?? true)}
              onChange={(e) => set(["decision", "allowNoAction"], e.target.checked)}
            />
            <span>允許不介入</span>
          </label>
        </div>
        <FieldIssues issues={issueFor("limits")} />
        <FieldIssues issues={issueFor("decision")} />
      </div>

      {issues.some((i) => i.field === "$") && (
        <p className="cap-card-error" role="alert">
          {issues.find((i) => i.field === "$")!.message}
        </p>
      )}

      <div className="row wrap" style={{ marginTop: 14 }}>
        <button onClick={save} disabled={saving}>
          {saving ? "儲存中…" : "儲存"}
        </button>
        <button onClick={onClose}>取消</button>
        {advanced && (
          <button
            onClick={async () => {
              const result = await api.recipeConvert(JSON.stringify(r), "yaml");
              setYamlView(result.valid ? result.text! : "（目前內容尚未通過驗證）");
            }}
          >
            檢視 YAML
          </button>
        )}
      </div>
      {yamlView && (
        <pre className="json-view" aria-label="YAML 內容">
          {yamlView}
        </pre>
      )}
    </Dialog>
  );
}

function TemplateList({
  templates,
  onChange,
}: {
  templates: string[];
  onChange: (t: string[]) => void;
}) {
  const [draft, setDraft] = React.useState("");
  return (
    <div className="template-list">
      {templates.map((t, i) => (
        <div className="row" key={i}>
          <input
            value={t}
            aria-label={`候選文字 ${i + 1}`}
            onChange={(e) => onChange(templates.map((x, j) => (j === i ? e.target.value : x)))}
          />
          <button
            aria-label={`上移候選文字 ${i + 1}`}
            disabled={i === 0}
            onClick={() => {
              const next = [...templates];
              [next[i - 1], next[i]] = [next[i], next[i - 1]];
              onChange(next);
            }}
          >
            ↑
          </button>
          <button
            aria-label={`刪除候選文字 ${i + 1}`}
            onClick={() => onChange(templates.filter((_, j) => j !== i))}
          >
            ✕
          </button>
        </div>
      ))}
      <div className="row">
        <input
          placeholder="新增一句候選文字…"
          value={draft}
          aria-label="新增候選文字"
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && draft.trim()) {
              onChange([...templates, draft.trim()]);
              setDraft("");
            }
          }}
        />
        <button
          disabled={!draft.trim()}
          onClick={() => {
            onChange([...templates, draft.trim()]);
            setDraft("");
          }}
        >
          新增
        </button>
      </div>
    </div>
  );
}

function FieldIssues({ issues }: { issues: { field: string; message: string }[] }) {
  if (issues.length === 0) return null;
  return (
    <ul className="field-issues" role="alert">
      {issues.map((i, idx) => (
        <li key={idx}>
          <code>{i.field}</code>：{i.message}
        </li>
      ))}
    </ul>
  );
}

// ---------------------------------------------------------------------------
// 模擬（含情境切換；後端保證零副作用）
// ---------------------------------------------------------------------------

const SCENARIOS: { key: string; label: string }[] = [
  { key: "quietHours", label: "安靜時段" },
  { key: "missingConsent", label: "缺少同意" },
  { key: "aiUnavailable", label: "AI 無法使用" },
  { key: "lowConfidence", label: "低信心訊號" },
  { key: "staleObservations", label: "資料過期" },
  { key: "recentlyFired", label: "最近已提醒過" },
  { key: "emergencyStop", label: "緊急停止中" },
];

function SimulateDialog({ recipeId, onClose }: { recipeId: string; onClose: () => void }) {
  const [flags, setFlags] = React.useState<Record<string, boolean>>({});
  const [report, setReport] = React.useState<ScenarioReport | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [running, setRunning] = React.useState(false);
  const { findCard } = useAppState();

  async function run() {
    setRunning(true);
    setError(null);
    try {
      const recipe = (await api.recipeGet(recipeId)) as RecipeJson;
      const receptor = recipe.trigger?.steps?.[0]?.receptor ?? "manual.event";
      const cond = recipe.trigger?.steps?.[0]?.condition ?? {};
      const scenario: Record<string, unknown> = { ...flags };
      scenario.event = { receptor, facts: cond };
      setReport(await api.recipeSimulateScenario(recipeId, scenario));
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  }

  return (
    <Dialog title="模擬這個自動互動" onClose={onClose}>
      <p className="muted small">模擬使用和真實執行相同的判斷邏輯，但保證不會真的執行任何動作。</p>
      <div className="row wrap" role="group" aria-label="模擬情境">
        {SCENARIOS.map((s) => (
          <label key={s.key} className={flags[s.key] ? "candidate on" : "candidate"}>
            <input
              type="checkbox"
              checked={Boolean(flags[s.key])}
              onChange={(e) => setFlags((f) => ({ ...f, [s.key]: e.target.checked }))}
            />
            {s.label}
          </label>
        ))}
      </div>
      <div className="row" style={{ margin: "10px 0" }}>
        <button onClick={run} disabled={running}>
          {running ? "模擬中…" : "開始模擬"}
        </button>
      </div>
      {error && <p className="cap-card-error" role="alert">{error}</p>}
      {report && (
        <div>
          <ol className="story-flow">
            {report.stages.map((stage, i) => (
              <li key={i}>
                <span className="story-label">{stageLabel(String(stage["stage"]))}</span>
                <span>{describeStage(stage, findCard)}</span>
              </li>
            ))}
          </ol>
          <p className="sim-verdict">
            {report.wouldExecute ? (
              <Badge kind="ok">會執行</Badge>
            ) : (
              <Badge kind="muted">不會執行</Badge>
            )}
            <span className="muted small">　{report.sideEffects}</span>
          </p>
        </div>
      )}
    </Dialog>
  );
}

function stageLabel(stage: string): string {
  switch (stage) {
    case "trigger":
      return "觸發";
    case "limits":
      return "頻率";
    case "consent":
      return "同意";
    case "aiGate":
      return "AI";
    case "planning":
      return "計畫";
    case "policy":
      return "安全";
    default:
      return stage;
  }
}

function describeStage(
  stage: Record<string, unknown>,
  findCard: (kind: "receptor" | "actuator" | "tool", id: string) => { name: string }
): string {
  const name = String(stage["stage"]);
  if (name === "trigger") return stage["ok"] ? "條件成立" : "條件不成立，這次不會觸發";
  if (name === "limits") return String(stage["detail"] ?? "");
  if (name === "consent") {
    const missing = (stage["missing"] as string[]) ?? [];
    return missing.length === 0 ? "同意檢查通過" : `缺少同意：${missing.join("、")}`;
  }
  if (name === "aiGate") {
    const d = stage["detail"] as Record<string, unknown> | undefined;
    return String(d?.["reason"] ?? aiOutcomeLabel(String(d?.["outcome"] ?? "")));
  }
  if (name === "planning") {
    const steps = (stage["steps"] as { actuatorId: string }[]) ?? [];
    if (steps.length === 0) return "沒有合適的回應方式（不介入）";
    return `準備使用：${steps.map((s) => findCard("actuator", s.actuatorId).name).join("、")}`;
  }
  if (name === "policy") {
    const steps = (stage["steps"] as { actuatorId: string; outcome: string }[]) ?? [];
    if (steps.length === 0) return "（沒有需要審核的步驟）";
    return steps
      .map(
        (s) =>
          `${findCard("actuator", s.actuatorId).name}：${
            s.outcome === "authorized" ? "允許" : s.outcome === "blocked" ? "阻止" : "需要確認"
          }`
      )
      .join("；");
  }
  return "";
}

function aiOutcomeLabel(outcome: string): string {
  switch (outcome) {
    case "disabled":
      return "這個配方不使用 AI";
    case "notNeeded":
      return "證據明確，不需要 AI";
    default:
      return outcome;
  }
}

// ---------------------------------------------------------------------------
// 影響預覽（啟用前）
// ---------------------------------------------------------------------------

function ImpactPreviewDialog({
  recipe,
  onCancel,
  onConfirm,
}: {
  recipe: RecipeJson;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { human, findCard } = useAppState();
  const receptorIds: string[] = [
    ...(recipe.trigger?.steps ?? []).map((s: RecipeJson) => String(s.receptor)),
    ...((recipe.context?.receptors as string[]) ?? []),
  ];
  const candidates: string[] = recipe.actuation?.candidates ?? [];
  const aiMode = recipe.ai?.mode ?? "never";

  const cardOf = (id: string): HumanCard | undefined =>
    human?.actuators.find((a) => a.id === id);
  const external = candidates.some((c) => cardOf(c)?.effect?.externalSideEffect === true);
  const physical = candidates.some((c) => cardOf(c)?.effect?.physicalEffect === true);
  const sensitiveIn = receptorIds.some((id) => {
    const card = human?.receptors.find((r) => r.id === id);
    return card?.data?.sensitivity === "high";
  });

  return (
    <Dialog title="啟用前的影響預覽" onClose={onCancel}>
      <div className="impact-preview">
        <h3>這個自動互動會：</h3>
        <ul className="impact-yes">
          {receptorIds.map((id) => (
            <li key={`r-${id}`}>✓ 讀取「{findCard("receptor", id).name}」</li>
          ))}
          {candidates.map((id) => (
            <li key={`a-${id}`}>✓ 可能使用「{findCard("actuator", id).name}」回應</li>
          ))}
          {aiMode !== "never" && (
            <li>
              ✓ 訊號模糊時會請 AI 協助（
              {recipe.ai?.onUnavailable === "no-action"
                ? "AI 沒回應就不介入"
                : "AI 沒回應改用本機規則"}
              ）
            </li>
          )}
        </ul>
        <h3>這個自動互動不會：</h3>
        <ul className="impact-no">
          {!sensitiveIn && <li>✗ 使用攝影機、麥克風或其他高敏感來源</li>}
          {!physical && <li>✗ 控制實體裝置</li>}
          {!external && <li>✗ 將資料傳送到外部服務</li>}
          {aiMode === "never" && <li>✗ 呼叫任何 AI</li>}
        </ul>
        <p className="muted small">執行時仍會再次經過安全規則與同意檢查；這裡只是事前預覽。</p>
      </div>
      <div className="row wrap">
        <button onClick={onConfirm}>確認啟用</button>
        <button onClick={onCancel}>先不要</button>
      </div>
    </Dialog>
  );
}
