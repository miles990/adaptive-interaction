// 對話框與二段式確認：Escape 關閉、焦點回復、estop 類操作不可一鍵誤觸。

import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ConfirmButton, Dialog } from "../components/Dialog";

describe("Dialog", () => {
  it("closes on Escape", async () => {
    const onClose = vi.fn();
    render(
      <Dialog title="測試視窗" onClose={onClose}>
        <button>內容按鈕</button>
      </Dialog>
    );
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("is announced as a modal dialog", () => {
    render(
      <Dialog title="測試視窗" onClose={() => {}}>
        x
      </Dialog>
    );
    const dialog = screen.getByRole("dialog", { name: "測試視窗" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
  });
});

describe("ConfirmButton", () => {
  it("requires a second, different click before firing", async () => {
    const onConfirm = vi.fn();
    render(<ConfirmButton label="緊急停止" confirmLabel="立即停止一切？" onConfirm={onConfirm} />);
    await userEvent.click(screen.getByRole("button", { name: "緊急停止" }));
    // 第一下絕不觸發。
    expect(onConfirm).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "立即停止一切？" }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it("Enter on the initial button only arms, never fires", async () => {
    const onConfirm = vi.fn();
    render(<ConfirmButton label="緊急停止" confirmLabel="立即停止一切？" onConfirm={onConfirm} />);
    screen.getByRole("button", { name: "緊急停止" }).focus();
    await userEvent.keyboard("{Enter}");
    expect(onConfirm).not.toHaveBeenCalled();
    // 鍵盤仍可完成第二步。
    screen.getByRole("button", { name: "立即停止一切？" }).focus();
    await userEvent.keyboard("{Enter}");
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it("can be cancelled after arming", async () => {
    const onConfirm = vi.fn();
    render(<ConfirmButton label="刪除" confirmLabel="確定刪除？" onConfirm={onConfirm} />);
    await userEvent.click(screen.getByRole("button", { name: "刪除" }));
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(onConfirm).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "刪除" })).toBeInTheDocument();
  });
});
