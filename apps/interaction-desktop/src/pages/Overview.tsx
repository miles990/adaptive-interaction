import React from "react";
import { api, Receipt, Session } from "../api";
import { Badge, JsonView, Section, StateView, statusBadgeKind, useAsync } from "../ui";

export function OverviewPage({ refreshKey }: { refreshKey: number }) {
  const [status] = useAsync(() => api.status(), [refreshKey]);
  const [session, reloadSession] = useAsync(() => api.sessionGet(), [refreshKey]);
  const [actions] = useAsync(() => api.actionsList(8), [refreshKey]);
  const [outbox] = useAsync(() => api.outbox(8), [refreshKey]);
  const [caps] = useAsync(() => api.capabilities(true), [refreshKey]);
  const [label, setLabel] = React.useState("desktop");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string>();

  async function startSession() {
    setBusy(true);
    setError(undefined);
    try {
      await api.sessionStart(label, []);
      reloadSession();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function stopSession() {
    setBusy(true);
    try {
      await api.sessionStop();
      reloadSession();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="grid-two">
      <Section title="Runtime 狀態">
        <StateView state={status}>
          {(s) => (
            <table className="kv">
              <tbody>
                <tr>
                  <td>版本</td>
                  <td>{String(s["version"])}</td>
                </tr>
                <tr>
                  <td>啟動時間</td>
                  <td>{String(s["startedAt"])}</td>
                </tr>
                <tr>
                  <td>緊急停止</td>
                  <td>
                    <Badge kind={s["emergencyStop"] ? "bad" : "ok"}>
                      {s["emergencyStop"] ? "啟動中" : "未啟動"}
                    </Badge>
                  </td>
                </tr>
                <tr>
                  <td>配方</td>
                  <td>{JSON.stringify(s["recipes"])}</td>
                </tr>
                <tr>
                  <td>設定錯誤</td>
                  <td>{(s["configErrors"] as string[])?.length ? <JsonView value={s["configErrors"]} /> : "無"}</td>
                </tr>
              </tbody>
            </table>
          )}
        </StateView>
      </Section>

      <Section title="Session">
        {error && <div className="state-box state-error">{error}</div>}
        <StateView state={session} empty="沒有進行中的 session。">
          {(s: Session) => (
            <div>
              <p>
                <Badge kind={statusBadgeKind(s.state)}>{s.state}</Badge>{" "}
                <code>{s.sessionId}</code> {s.label ? `(${s.label})` : ""}
              </p>
              <p>同意範圍：</p>
              {s.consents.filter((c) => !c.revokedAt).length === 0 ? (
                <div className="state-box">尚未授予任何同意。</div>
              ) : (
                <ul>
                  {s.consents
                    .filter((c) => !c.revokedAt)
                    .map((c, i) => (
                      <li key={i}>
                        <code>
                          {c.scope.kind}:{c.scope.id}
                        </code>
                      </li>
                    ))}
                </ul>
              )}
              <button disabled={busy} onClick={stopSession}>
                結束 session
              </button>
            </div>
          )}
        </StateView>
        {!session.loading && !session.data && (
          <div className="row">
            <input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="label" />
            <button disabled={busy} onClick={startSession}>
              開始 session
            </button>
          </div>
        )}
      </Section>

      <Section title="能力摘要">
        <StateView state={caps}>
          {(c) => (
            <div>
              <p>
                受器 {c.receptors.length} · 動器 {c.actuators.length} · 工具{" "}
                {c.toolOperations.length} · snapshot v{c.version}
              </p>
              {c.constraints.length > 0 && (
                <ul>
                  {c.constraints.map((k, i) => (
                    <li key={i}>
                      <Badge kind="warn">{k.kind}</Badge> {k.detail}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </StateView>
      </Section>

      <Section title="最近訊息（conversation / web-ui）">
        <StateView state={outbox} empty="還沒有任何互動訊息。">
          {(messages) => (
            <ul className="feed">
              {[...messages].reverse().map((m, i) => (
                <li key={i}>
                  <Badge kind="info">{m.channel}</Badge>{" "}
                  <span className="muted">{m.intent}</span>{" "}
                  {m.text ?? <em className="muted">（刻意沉默）</em>}
                </li>
              ))}
            </ul>
          )}
        </StateView>
      </Section>

      <Section title="最近動作">
        <StateView state={actions} empty="還沒有任何動作。">
          {(receipts: Receipt[]) => (
            <table className="list">
              <thead>
                <tr>
                  <th>intent</th>
                  <th>actuator</th>
                  <th>狀態</th>
                  <th>驗證</th>
                </tr>
              </thead>
              <tbody>
                {receipts.map((r) => (
                  <tr key={r.actionId}>
                    <td>{r.intent}</td>
                    <td>
                      <code>{r.actuatorId}</code>
                    </td>
                    <td>
                      <Badge kind={statusBadgeKind(r.currentStatus)}>{r.currentStatus}</Badge>
                    </td>
                    <td>{r.verification?.verdict ?? "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </StateView>
      </Section>
    </div>
  );
}
