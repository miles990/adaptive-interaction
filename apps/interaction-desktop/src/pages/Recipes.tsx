import React from "react";
import { api } from "../api";
import { Badge, JsonView, Section, StateView, useAsync } from "../ui";

const TEMPLATE = `id: my-recipe
name: 新配方
enabled: false
trigger:
  mode: any
  steps:
    - receptor: manual.event
      condition:
        event: demo
decision:
  objective: respond-minimally
  allowNoAction: true
intent: acknowledge
message:
  mode: adaptive
  allowSilence: true
actuation:
  mode: adaptive
  candidates: [conversation, web-ui]
  minChannels: 0
  maxChannels: 2
limits:
  cooldown: 5m
  expiresAfter: 30s
`;

export function RecipesPage({ refreshKey }: { refreshKey: number }) {
  const [recipes, reload] = useAsync(() => api.recipesList(), [refreshKey]);
  const [editor, setEditor] = React.useState(TEMPLATE);
  const [validation, setValidation] = React.useState<unknown>(null);
  const [result, setResult] = React.useState<unknown>(null);
  const [error, setError] = React.useState<string>();

  async function validate() {
    setValidation(await api.recipeValidate(editor));
  }

  async function apply() {
    setError(undefined);
    try {
      await api.recipeUpsert(editor);
      reload();
      setResult({ applied: true });
    } catch (e) {
      setError(String(e));
    }
  }

  async function act(kind: "simulate" | "run" | "enable" | "disable" | "delete", id: string) {
    setError(undefined);
    setResult(null);
    try {
      switch (kind) {
        case "simulate":
          setResult(await api.recipeSimulate(id));
          break;
        case "run":
          setResult(await api.recipeRun(id));
          break;
        case "enable":
          await api.recipeSetEnabled(id, true);
          break;
        case "disable":
          await api.recipeSetEnabled(id, false);
          break;
        case "delete":
          await api.recipeDelete(id);
          break;
      }
      reload();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="grid-two">
      <Section title="已安裝配方">
        {error && <div className="state-box state-error">{error}</div>}
        <StateView state={recipes} empty="還沒有配方；用右側編輯器建立一個。">
          {(list) => (
            <table className="list">
              <thead>
                <tr>
                  <th>id</th>
                  <th>狀態</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {list.map((entry, i) => {
                  const r = entry.recipe as { id: string; name: string; enabled: boolean };
                  return (
                    <tr key={i}>
                      <td>
                        <code>{r.id}</code>
                        <div className="muted small">{r.name}</div>
                      </td>
                      <td>
                        <Badge kind={r.enabled ? "ok" : "muted"}>
                          {r.enabled ? "啟用" : "停用"}
                        </Badge>
                        <div className="muted small">
                          fired: {String((entry.state as Record<string, unknown>)["executionsThisSession"] ?? 0)}
                        </div>
                      </td>
                      <td className="row wrap">
                        <button onClick={() => act("simulate", r.id)}>模擬</button>
                        <button onClick={() => act("run", r.id)}>執行</button>
                        {r.enabled ? (
                          <button onClick={() => act("disable", r.id)}>停用</button>
                        ) : (
                          <button onClick={() => act("enable", r.id)}>啟用</button>
                        )}
                        <button className="danger" onClick={() => act("delete", r.id)}>
                          刪除
                        </button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </StateView>
        {result != null && (
          <>
            <h3>結果（含觸發原因／policy 決策）</h3>
            <JsonView value={result} />
          </>
        )}
      </Section>
      <Section
        title="配方編輯器（YAML）"
        actions={
          <span className="row">
            <button onClick={validate}>驗證</button>
            <button onClick={apply}>套用</button>
          </span>
        }
      >
        <textarea
          className="editor"
          value={editor}
          onChange={(e) => setEditor(e.target.value)}
          spellCheck={false}
        />
        {validation != null && <JsonView value={validation} />}
      </Section>
    </div>
  );
}
