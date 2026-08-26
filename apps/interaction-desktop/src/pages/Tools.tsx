import React from "react";
import { api, ToolManifest } from "../api";
import { Badge, JsonView, Section, StateView, useAsync } from "../ui";

const FORMATS = ["openai", "anthropic", "gemini", "openapi", "json-schema"];

export function ToolsPage() {
  const [tools] = useAsync(() => api.toolsList(), []);
  const [selected, setSelected] = React.useState<ToolManifest | null>(null);
  const [exportFormat, setExportFormat] = React.useState("openai");
  const [exported, setExported] = React.useState<unknown>(null);

  async function doExport() {
    setExported("產生中…");
    try {
      setExported(await api.toolsExport(exportFormat));
    } catch (e) {
      setExported({ error: String(e) });
    }
  }

  return (
    <div className="grid-two">
      <Section title="Tool Operations">
        <StateView state={tools} empty="沒有已註冊的工具。">
          {(list) => (
            <table className="list">
              <thead>
                <tr>
                  <th>名稱</th>
                  <th>角色</th>
                  <th>風險</th>
                </tr>
              </thead>
              <tbody>
                {list.map((t) => (
                  <tr
                    key={t.name}
                    className={selected?.name === t.name ? "selected" : ""}
                    onClick={() => setSelected(t)}
                  >
                    <td>
                      <code>{t.name}</code>
                    </td>
                    <td>{t.roles.join(", ")}</td>
                    <td>
                      <Badge kind={t.risk === "read-only" ? "ok" : "warn"}>{t.risk}</Badge>
                      {t.requiresApproval && <Badge kind="bad">需批准</Badge>}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </StateView>
      </Section>
      <Section
        title="Schema / 匯出"
        actions={
          <span className="row">
            <select value={exportFormat} onChange={(e) => setExportFormat(e.target.value)}>
              {FORMATS.map((f) => (
                <option key={f} value={f}>
                  {f}
                </option>
              ))}
            </select>
            <button onClick={doExport}>由同一 Canonical Manifest 匯出</button>
          </span>
        }
      >
        {selected ? (
          <div>
            <p className="muted">{selected.description}</p>
            <h3>Input schema</h3>
            <JsonView value={selected.inputSchema} />
            <h3>Output schema</h3>
            <JsonView value={selected.outputSchema} />
          </div>
        ) : exported ? (
          <JsonView value={exported} />
        ) : (
          <div className="state-box">點選一個工具檢視 schema，或選擇格式匯出。</div>
        )}
      </Section>
    </div>
  );
}
