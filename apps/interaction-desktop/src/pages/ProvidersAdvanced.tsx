// 進階：Provider Registry 原始檢視（完整 descriptor JSON）。

import { api } from "../api";
import { JsonView, Section, StateView, useAsync } from "../ui";

export function ProvidersAdvancedPage({ refreshKey }: { refreshKey: number }) {
  const [data] = useAsync(
    () => api.providersList() as Promise<Record<string, unknown>[]>,
    [refreshKey]
  );
  return (
    <Section title="Provider Registry（原始）">
      <StateView state={data} empty="沒有 provider。">
        {(list) => (
          <div>
            {list.map((p) => {
              const id = String((p.identity as Record<string, unknown>)?.id ?? "");
              return (
                <details key={id}>
                  <summary>
                    {id} — {String(p.state)}
                  </summary>
                  <JsonView value={p} />
                </details>
              );
            })}
          </div>
        )}
      </StateView>
    </Section>
  );
}
