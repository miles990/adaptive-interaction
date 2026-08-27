// 進階：Knowledge Graph 原始檢視（節點＋鄰接展開＋檢索除錯）。

import React from "react";
import { api } from "../api";
import { JsonView, Section, StateView, useAsync } from "../ui";

export function KnowledgeAdvancedPage({ refreshKey }: { refreshKey: number }) {
  const [data] = useAsync(() => api.knowledgeList(undefined, 200), [refreshKey]);
  const [graph, setGraph] = React.useState<Record<string, unknown> | null>(null);
  const [query, setQuery] = React.useState("");
  const [search, setSearch] = React.useState<Record<string, unknown> | null>(null);
  return (
    <div>
      <Section title="檢索除錯（FTS＋lexical-vector 候選）">
        <div className="row wrap">
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="query…" />
          <button
            disabled={!query.trim()}
            onClick={async () => setSearch(await api.knowledgeSearch(query, 20))}
          >
            搜尋
          </button>
        </div>
        {search && <JsonView value={search} />}
      </Section>
      <Section title="Knowledge Graph（原始節點）">
        <StateView state={data} empty="沒有知識節點。">
          {(d) => (
            <div>
              {((d.nodes as Record<string, unknown>[] | undefined) ?? []).map((n) => (
                <details key={String(n.nodeId)}>
                  <summary>
                    {String(n.title)} — {String(n.status)}
                    <button
                      style={{ marginLeft: 8 }}
                      onClick={async (e) => {
                        e.preventDefault();
                        setGraph(await api.knowledgeGraph(String(n.nodeId)));
                      }}
                    >
                      展開鄰接
                    </button>
                  </summary>
                  <JsonView value={n} />
                </details>
              ))}
            </div>
          )}
        </StateView>
        {graph && (
          <div className="state-box">
            <strong>鄰接展開</strong>
            <JsonView value={graph} />
          </div>
        )}
      </Section>
    </div>
  );
}
