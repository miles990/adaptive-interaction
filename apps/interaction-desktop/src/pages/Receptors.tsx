import React from "react";
import { api } from "../api";
import { Badge, JsonView, Section, StateView, statusBadgeKind, useAsync } from "../ui";

export function ReceptorsPage({ refreshKey }: { refreshKey: number }) {
  const [caps, reload] = useAsync(() => api.capabilities(true), [refreshKey]);
  const [preview, setPreview] = React.useState<{ id: string; value: unknown } | null>(null);
  const [error, setError] = React.useState<string>();

  async function toggle(id: string, enabled: boolean) {
    setError(undefined);
    try {
      await api.setReceptorEnabled(id, enabled);
      reload();
    } catch (e) {
      setError(String(e));
    }
  }

  async function test(id: string) {
    setError(undefined);
    try {
      const obs = await api.testReceptor(id);
      setPreview({ id, value: obs });
    } catch (e) {
      setPreview({ id, value: { error: String(e) } });
    }
  }

  return (
    <div>
      {error && <div className="state-box state-error">{error}</div>}
      <Section title="受器（Receptors）">
        <StateView state={caps} empty="沒有已註冊的受器。">
          {(c) => (
            <table className="list">
              <thead>
                <tr>
                  <th>id</th>
                  <th>類別</th>
                  <th>模式</th>
                  <th>敏感度</th>
                  <th>狀態</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {c.receptors.map((r) => (
                  <tr key={r.id}>
                    <td>
                      <code>{r.id}</code>
                      <div className="muted small">{r.description}</div>
                    </td>
                    <td>{r.category}</td>
                    <td>{r.mode}</td>
                    <td>
                      {r.sensitivity}
                      {r.requiresConsent && <Badge kind="warn">需同意</Badge>}
                    </td>
                    <td>
                      <Badge kind={statusBadgeKind(r.availability)}>{r.availability}</Badge>
                    </td>
                    <td className="row">
                      {r.availability === "disabled" ? (
                        <button onClick={() => toggle(r.id, true)}>啟用</button>
                      ) : (
                        <button onClick={() => toggle(r.id, false)}>停用</button>
                      )}
                      <button onClick={() => test(r.id)}>測試讀取</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </StateView>
      </Section>
      {preview && (
        <Section title={`觀察預覽：${preview.id}（facts 與 inferences 分離）`}>
          <JsonView value={preview.value} />
        </Section>
      )}
    </div>
  );
}
