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
      // 失敗時保留上一次的資料：StateView 會顯示「更新失敗（顯示的是上一次的資料）」
      // 而不是把整個清單換成錯誤框，使用者展開中的內容不會憑空消失。
      .catch((e) => alive && setState((s) => ({ loading: false, error: String(e), data: s.data })));
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
  // 背景重新整理（已有資料、loading 再次為 true）時**不得**把內容換成「載入中…」：
  // 那會讓底下的元件整個卸載重掛，使用者展開的面板（例如工作卡的訊息、核可
  // 按鈕的裁決結果）會在每一次 SSE 事件觸發的刷新時收合／消失。只有第一次
  // 載入（還沒有任何資料）才顯示載入中；之後保留舊資料並以 aria-busy 標示更新中。
  const data = state.data;
  const hasData = data !== undefined && data !== null;
  if (state.loading && !hasData) return <div className="state-box">載入中…</div>;
  if (state.error && !hasData)
    return (
      <div className="state-box state-error">
        錯誤：{state.error}
      </div>
    );
  if (state.error && hasData) {
    return (
      <div aria-busy={state.loading || undefined}>
        <div className="state-box state-error" role="alert">
          更新失敗：{state.error}（顯示的是上一次的資料）
        </div>
        {Array.isArray(data) && data.length === 0 ? (
          <div className="state-box">{empty ?? "目前沒有資料。"}</div>
        ) : (
          children(data as NonNullable<T>)
        )}
      </div>
    );
  }
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
