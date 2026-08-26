import React from "react";
import { RuntimeEvent } from "../api";
import { Badge, JsonView, Section } from "../ui";

// Timeline: Observation → Plan → Policy Decision → Bounded Action → Execution
// → Receipt → Verification → Adaptation, grouped by correlation id.

function eventKind(t: string): string {
  if (t.startsWith("receptor.")) return "info";
  if (t === "plan.blocked" || t.startsWith("action.failed")) return "bad";
  if (t === "emergency.stop") return "bad";
  if (t === "action.completed") return "ok";
  if (t === "action.uncertain") return "warn";
  if (t.startsWith("action.")) return "pending";
  return "muted";
}

export function TimelinePage({ events }: { events: RuntimeEvent[] }) {
  const [selected, setSelected] = React.useState<RuntimeEvent | null>(null);
  const [filter, setFilter] = React.useState("");

  const filtered = events.filter(
    (e) =>
      !filter ||
      e.eventType.includes(filter) ||
      (e.correlationId ?? "").includes(filter) ||
      JSON.stringify(e.payload).includes(filter)
  );

  return (
    <div className="grid-two">
      <Section
        title={`即時時間軸（${events.length} 事件）`}
        actions={
          <input
            placeholder="篩選 (事件類型 / correlation / 內容)"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        }
      >
        {filtered.length === 0 ? (
          <div className="state-box">
            尚無事件。開始一個 session、推入觀察或執行計畫後，
            Observation → Plan → Policy → Action → Receipt → Verification 會即時顯示在這裡。
          </div>
        ) : (
          <ul className="timeline">
            {[...filtered].reverse().slice(0, 100).map((e) => (
              <li
                key={`${e.sequence}`}
                onClick={() => setSelected(e)}
                className={selected?.sequence === e.sequence ? "selected" : ""}
              >
                <span className="muted small">#{e.sequence}</span>{" "}
                <Badge kind={eventKind(e.eventType)}>{e.eventType}</Badge>{" "}
                <span className="small">{new Date(e.timestamp).toLocaleTimeString()}</span>
                {e.correlationId && (
                  <span className="muted small"> corr:{e.correlationId.slice(0, 13)}…</span>
                )}
              </li>
            ))}
          </ul>
        )}
      </Section>
      <Section title="事件內容">
        {selected ? (
          <JsonView value={selected} />
        ) : (
          <div className="state-box">點選左側事件檢視完整內容（含 policy 決策與收據狀態）。</div>
        )}
      </Section>
    </div>
  );
}
