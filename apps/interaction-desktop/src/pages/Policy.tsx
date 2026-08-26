import React from "react";
import { api, Session } from "../api";
import { Badge, JsonView, Section, StateView, useAsync } from "../ui";

export function PolicyPage({ refreshKey }: { refreshKey: number }) {
  const [policy, reloadPolicy] = useAsync(() => api.policyGet(), [refreshKey]);
  const [session, reloadSession] = useAsync(() => api.sessionGet(), [refreshKey]);
  const [scope, setScope] = React.useState("channel:haptic");
  const [patchText, setPatchText] = React.useState('{\n  "initiative": "active"\n}');
  const [error, setError] = React.useState<string>();

  async function applyPatch() {
    setError(undefined);
    try {
      await api.policyPatch(JSON.parse(patchText));
      reloadPolicy();
    } catch (e) {
      setError(String(e));
    }
  }

  async function grant() {
    setError(undefined);
    try {
      await api.consentGrant(scope);
      reloadSession();
    } catch (e) {
      setError(String(e));
    }
  }

  async function revoke(s: string) {
    setError(undefined);
    try {
      await api.consentRevoke(s);
      reloadSession();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="grid-two">
      <Section title="有效政策（Rust Governor 強制執行）">
        {error && <div className="state-box state-error">{error}</div>}
        <StateView state={policy}>{(p) => <JsonView value={p} />}</StateView>
      </Section>
      <div>
        <Section title="政策修改（JSON merge patch）">
          <textarea
            className="editor small-editor"
            value={patchText}
            onChange={(e) => setPatchText(e.target.value)}
            spellCheck={false}
          />
          <div className="row">
            <button onClick={applyPatch}>套用 patch</button>
          </div>
          <p className="muted small">
            隱藏 UI 按鈕不構成安全控制：所有值最終由後端 min() 限界；
            resumeHighRiskAfterRestart 永遠釘死為 false。
          </p>
        </Section>
        <Section title="Session 同意">
          <StateView state={session} empty="沒有進行中的 session；先在總覽頁開始。">
            {(s: Session) => (
              <div>
                <ul>
                  {s.consents
                    .filter((c) => !c.revokedAt)
                    .map((c, i) => {
                      const scopeStr = `${c.scope.kind === "toolOperation" ? "tool" : c.scope.kind}:${c.scope.id}`;
                      return (
                        <li key={i} className="row">
                          <Badge kind="ok">{scopeStr}</Badge>
                          <button className="danger" onClick={() => revoke(scopeStr)}>
                            撤回
                          </button>
                        </li>
                      );
                    })}
                </ul>
                <div className="row">
                  <input value={scope} onChange={(e) => setScope(e.target.value)} />
                  <button onClick={grant}>授予同意</button>
                </div>
                <p className="muted small">
                  範例：channel:haptic、actuator:mock.actuator。撤回會立即取消範圍內執行中的動作。
                </p>
              </div>
            )}
          </StateView>
        </Section>
      </div>
    </div>
  );
}
