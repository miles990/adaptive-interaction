// regression（Playwright work-delegate「拒絕」）：StateView 在**背景重新整理**時把內容換成
// 「載入中…」，底下的卡片整個卸載重掛，使用者剛展開的訊息面板與核可裁決結果在每一次
// SSE 事件觸發的刷新時消失。只有第一次載入（沒有任何資料）才可以顯示載入中。
import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { StateView } from "../ui";

function Card({ label }: { label: string }) {
  const [open, setOpen] = React.useState(false);
  return (
    <div>
      <span>{label}</span>
      <button onClick={() => setOpen((v) => !v)}>{open ? "收合" : "展開"}</button>
      {open && <p>展開中的內容</p>}
    </div>
  );
}

describe("StateView 背景重新整理不卸載內容", () => {
  it("第一次載入（沒有資料）才顯示載入中", () => {
    render(<StateView state={{ loading: true }}>{() => <Card label="x" />}</StateView>);
    expect(screen.getByText("載入中…")).toBeTruthy();
  });

  it("已有資料時 loading 再次為 true 不得換成載入中，展開狀態要保留", () => {
    const items = [{ id: "s1" }];
    const view = (loading: boolean) => (
      <StateView state={{ loading, data: items }}>
        {(list) => (
          <>
            {list.map((s) => (
              <Card key={s.id} label={s.id} />
            ))}
          </>
        )}
      </StateView>
    );
    const { rerender } = render(view(false));
    fireEvent.click(screen.getByRole("button", { name: "展開" }));
    expect(screen.getByText("展開中的內容")).toBeTruthy();
    rerender(view(true));
    expect(screen.queryByText("載入中…")).toBeNull();
    expect(screen.getByText("展開中的內容")).toBeTruthy();
    expect(screen.getByRole("button", { name: "收合" })).toBeTruthy();
  });

  it("更新失敗但已有資料：顯示更新失敗並保留舊資料，不清空畫面", () => {
    render(
      <StateView state={{ loading: false, error: "boom", data: [{ id: "s1" }] }}>
        {(list) => <>{list.map((s) => <Card key={s.id} label={s.id} />)}</>}
      </StateView>
    );
    expect(screen.getByRole("alert").textContent).toContain("更新失敗");
    expect(screen.getByText("s1")).toBeTruthy();
  });

  it("沒有資料的錯誤照舊顯示錯誤框", () => {
    render(<StateView state={{ loading: false, error: "boom" }}>{() => <Card label="x" />}</StateView>);
    expect(screen.getByText(/錯誤：boom/)).toBeTruthy();
  });
});
