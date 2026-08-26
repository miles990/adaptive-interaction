// 感知來源／回應方式／工具操作 三頁共用的能力卡片列表。
// 卡片內容完全來自後端解析器（四層優先鏈＋保守 unknown）。

import React from "react";
import { HumanCard } from "../api";
import { useAppState } from "../appstate";
import { CapabilityCard } from "../components/CapabilityCard";

export function CapabilitiesPage({
  kind,
  advanced,
}: {
  kind: "receptor" | "actuator" | "tool-operation";
  advanced: boolean;
}) {
  const { human, humanError, refreshHuman } = useAppState();
  const [filter, setFilter] = React.useState("");
  const [category, setCategory] = React.useState("all");

  if (humanError) {
    return (
      <div className="state-box state-error">
        無法載入能力清單：{humanError}
        <div style={{ marginTop: 8 }}>
          <button onClick={() => refreshHuman()}>重試</button>
        </div>
      </div>
    );
  }
  if (!human) return <div className="state-box">載入中…</div>;

  const cards: HumanCard[] =
    kind === "receptor"
      ? human.receptors
      : kind === "actuator"
        ? human.actuators
        : human.toolOperations;

  const categories = Array.from(new Set(cards.map((c) => c.category))).sort();
  const visible = cards.filter((c) => {
    if (category !== "all" && c.category !== category) return false;
    if (!filter.trim()) return true;
    const q = filter.trim().toLowerCase();
    return (
      c.displayName.toLowerCase().includes(q) ||
      c.id.toLowerCase().includes(q) ||
      (c.shortDescription ?? "").toLowerCase().includes(q)
    );
  });

  const intro =
    kind === "receptor"
      ? "感知來源是系統可以接收的資訊 — 啟用它，系統才「知道」這件事。"
      : kind === "actuator"
        ? "回應方式是系統可以採取的行動 — 每一種都受安全規則與同意限制。"
        : "工具操作是 AI 可以讀取、建立或修改的軟體能力 — 依風險分級管理。";

  return (
    <div>
      <p className="page-intro">{intro}</p>
      <div className="row wrap" style={{ marginBottom: 12 }}>
        <input
          type="search"
          placeholder="搜尋…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          aria-label="搜尋能力"
        />
        <select value={category} onChange={(e) => setCategory(e.target.value)} aria-label="分類篩選">
          <option value="all">全部分類</option>
          {categories.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
        <span className="muted small">{visible.length} 項</span>
      </div>
      {visible.length === 0 ? (
        <div className="state-box">沒有符合的項目。</div>
      ) : (
        <div className="card-grid">
          {visible.map((c) => (
            <CapabilityCard key={c.id} card={c} advanced={advanced} onChanged={refreshHuman} />
          ))}
        </div>
      )}
    </div>
  );
}
