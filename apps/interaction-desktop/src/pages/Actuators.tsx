import React from "react";
import { api, Receipt } from "../api";
import { Badge, JsonView, Section, StateView, statusBadgeKind, useAsync } from "../ui";

export function ActuatorsPage({ refreshKey }: { refreshKey: number }) {
  const [caps, reload] = useAsync(() => api.capabilities(true), [refreshKey]);
  const [testResult, setTestResult] = React.useState<{ id: string; value: unknown } | null>(null);
  const [error, setError] = React.useState<string>();

  async function toggle(id: string, enabled: boolean) {
    setError(undefined);
    try {
      await api.setActuatorEnabled(id, enabled);
      reload();
    } catch (e) {
      setError(String(e));
    }
  }

  async function test(id: string) {
    setError(undefined);
    setTestResult({ id, value: "執行中（完整通過 policy 授權路徑）…" });
    try {
      const receipts: Receipt[] = await api.testActuator(id);
      setTestResult({ id, value: receipts });
    } catch (e) {
      setTestResult({ id, value: { error: String(e) } });
    }
  }

  return (
    <div>
      {error && <div className="state-box state-error">{error}</div>}
      <Section title="動器（Actuators）— 測試不會繞過 policy">
        <StateView state={caps} empty="沒有已註冊的動器。">
          {(c) => (
            <table className="list">
              <thead>
                <tr>
                  <th>id</th>
                  <th>通道</th>
                  <th>風險</th>
                  <th>限制</th>
                  <th>狀態</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {c.actuators.map((a) => (
                  <tr key={a.id}>
                    <td>
                      <code>{a.id}</code>
                      <div className="muted small">{a.description}</div>
                    </td>
                    <td>{a.channel}</td>
                    <td>
                      <Badge kind={a.riskClass === "low" || a.riskClass === "read-only" ? "ok" : "warn"}>
                        {a.riskClass}
                      </Badge>
                      {a.externalSideEffect && <Badge kind="bad">外部副作用</Badge>}
                      {a.requiresConsent && <Badge kind="warn">需同意</Badge>}
                    </td>
                    <td className="small">
                      {a.limits?.maxMagnitude != null && <div>maxMag {String(a.limits.maxMagnitude)}</div>}
                      {a.limits?.maxDurationMs != null && <div>maxDur {String(a.limits.maxDurationMs)}ms</div>}
                    </td>
                    <td>
                      <Badge kind={statusBadgeKind(a.availability)}>{a.availability}</Badge>
                    </td>
                    <td className="row">
                      {a.availability === "disabled" ? (
                        <button onClick={() => toggle(a.id, true)}>啟用</button>
                      ) : (
                        <button onClick={() => toggle(a.id, false)}>停用</button>
                      )}
                      <button onClick={() => test(a.id)}>測試</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </StateView>
      </Section>
      {testResult && (
        <Section title={`測試結果：${testResult.id}（收據含 policy decisions）`}>
          <JsonView value={testResult.value} />
        </Section>
      )}
    </div>
  );
}
