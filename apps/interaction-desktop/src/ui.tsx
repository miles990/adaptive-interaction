// Shared UI atoms: badges, sections, async-state wrappers. Every list view
// gets explicit loading / empty / error / offline handling for free.

import React from "react";

export function Badge({ kind, children }: { kind: string; children: React.ReactNode }) {
  return <span className={`badge badge-${kind}`}>{children}</span>;
}

export function statusBadgeKind(status: string): string {
  switch (status) {
    case "completed":
    case "healthy":
    case "available":
    case "active":
      return "ok";
    case "accepted":
    case "dispatched":
    case "authorized":
    case "planned":
      return "pending";
    case "acknowledged":
    case "observed":
    case "degraded":
      return "info";
    case "blocked":
    case "failed":
    case "unhealthy":
    case "revoked":
      return "bad";
    case "uncertain":
    case "unknown":
      return "warn";
    case "cancelled":
    case "expired":
    case "stopped":
    case "offline":
    case "disabled":
    case "consent-required":
      return "muted";
    default:
      return "muted";
  }
}

export function Section({
  title,
  actions,
  children,
}: {
  title: string;
  actions?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="section">
      <div className="section-head">
        <h2>{title}</h2>
        <div className="section-actions">{actions}</div>
      </div>
      {children}
    </section>
  );
}

export interface AsyncState<T> {
  loading: boolean;
  error?: string;
  data?: T;
}

export function useAsync<T>(fn: () => Promise<T>, deps: React.DependencyList): [AsyncState<T>, () => void] {
  const [state, setState] = React.useState<AsyncState<T>>({ loading: true });
  const [tick, setTick] = React.useState(0);
  React.useEffect(() => {
    let alive = true;
    setState((s) => ({ ...s, loading: true }));
    fn()
      .then((data) => alive && setState({ loading: false, data }))
      .catch((e) => alive && setState({ loading: false, error: String(e) }));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, tick]);
  return [state, () => setTick((t) => t + 1)];
}

export function StateView<T>({
  state,
  empty,
  children,
}: {
  state: AsyncState<T>;
  empty?: string;
  children: (data: NonNullable<T>) => React.ReactNode;
}) {
  if (state.loading) return <div className="state-box">載入中…</div>;
  if (state.error)
    return (
      <div className="state-box state-error">
        錯誤：{state.error}
      </div>
    );
  const data = state.data;
  if (
    data === undefined ||
    data === null ||
    (Array.isArray(data) && data.length === 0)
  )
    return <div className="state-box">{empty ?? "目前沒有資料。"}</div>;
  return <>{children(data as NonNullable<T>)}</>;
}

export function JsonView({ value }: { value: unknown }) {
  return <pre className="json-view">{JSON.stringify(value, null, 2)}</pre>;
}

export function Toggle({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label?: string;
}) {
  return (
    <label className="toggle">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span>{label ?? (checked ? "啟用" : "停用")}</span>
    </label>
  );
}
