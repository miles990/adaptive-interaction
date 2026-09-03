// v0.5.1 §15 記憶匯出：範圍要正反兩面明列，還原的拒絕路徑要真的拒絕。
//
// - 匯出的範圍不能只在靜態文案裡寫死：後端 `included`／`notIncluded` 說了什麼，
//   畫面就照實列什麼；後端沒說的（舊版回應）不得由前端補一份好看的清單。
// - 這裡只匯出記憶，不是完整備份——文案不得出現「完整備份」式的宣稱。
// - 還原的 5 MiB／1,000 條上限是拒絕，不是截斷：拒絕時一筆都不能寫進去。
// - 還原中途失敗要照實說已經寫進去幾筆，不假裝整批原子性。

import { describe, expect, it, vi, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { api } from "../api";
import {
  BackupSection,
  MAX_BACKUP_BYTES,
  MAX_BACKUP_ITEMS,
  scopeLabels,
} from "../pages/BackupSection";

afterEach(() => {
  vi.restoreAllMocks();
});

/** 造一個「號稱很大」的檔：不真的配置 5 MiB，只讓 size 超標。 */
function oversizedFile(): File {
  const file = new File(["{}"], "memory-export.json", { type: "application/json" });
  Object.defineProperty(file, "size", { value: MAX_BACKUP_BYTES + 1 });
  return file;
}

function backupFile(items: unknown[]): File {
  return new File([JSON.stringify({ count: items.length, items })], "memory-export.json", {
    type: "application/json",
  });
}

describe("匯出範圍：included／notIncluded 明列", () => {
  it("靜態文案就說清楚只含記憶，並逐項點名沒有的東西", () => {
    const { container } = render(<BackupSection />);
    const text = container.textContent ?? "";
    expect(text).toContain("只含記憶");
    expect(text).toContain("知識節點");
    expect(text).toContain("素材與衍生物");
    expect(text).toContain("知識的來源紀錄");
    expect(text).toContain("互動記憶");
    // 靜態說明不得自稱「完整備份」——「不是完整備份」只會在後端說達到上限時
    // 以警告出現（見 regressions-review2-memory），不是常駐宣稱。
    expect(text).not.toContain("完整備份");
  });

  it("匯出後照後端回的 included／notIncluded 明列範圍（不是前端寫死）", async () => {
    vi.spyOn(api, "memoryExport").mockResolvedValue({
      count: 3,
      total: 3,
      limit: 1000,
      limitReached: false,
      scope: "memory-items-only",
      included: ["memory-items"],
      notIncluded: [
        "knowledge-nodes",
        "assets-and-derivatives",
        "knowledge-receipts",
        "character-interaction-memory",
      ],
      items: [],
    });
    render(<BackupSection />);
    await userEvent.click(screen.getByRole("button", { name: "匯出記憶" }));
    const included = await screen.findByTestId("export-included");
    expect(included).toHaveTextContent("包含：記憶項目");
    const notIncluded = screen.getByTestId("export-not-included");
    expect(notIncluded).toHaveTextContent("知識節點");
    expect(notIncluded).toHaveTextContent("素材與衍生物");
    expect(notIncluded).toHaveTextContent("知識的來源紀錄");
    expect(notIncluded).toHaveTextContent("角色互動記憶");
  });

  it("後端沒回範圍欄位時不編造清單", async () => {
    vi.spyOn(api, "memoryExport").mockResolvedValue({ count: 1, items: [{ title: "x" }] });
    render(<BackupSection />);
    await userEvent.click(screen.getByRole("button", { name: "匯出記憶" }));
    await screen.findByText("匯出結果");
    expect(screen.queryByTestId("export-included")).not.toBeInTheDocument();
    expect(screen.queryByTestId("export-not-included")).not.toBeInTheDocument();
  });

  it("scopeLabels：認得的鍵翻成人話，不認得的照原樣列出（不吞掉）", () => {
    expect(scopeLabels(["memory-items", "knowledge-nodes"])).toEqual(["記憶項目", "知識節點"]);
    expect(scopeLabels(["something-new"])).toEqual(["something-new"]);
    expect(scopeLabels(undefined)).toEqual([]);
    expect(scopeLabels(["", 42])).toEqual([]);
  });
});

describe("還原的拒絕路徑：拒絕就是一筆都不寫", () => {
  it("超過 5 MiB 直接拒絕，不呼叫任何寫入 API", async () => {
    const create = vi.spyOn(api, "memoryCreate").mockResolvedValue({});
    render(<BackupSection />);
    await userEvent.upload(screen.getByLabelText("選擇記憶備份檔"), oversizedFile());
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/5 MiB/);
    expect(create).not.toHaveBeenCalled();
  });

  it("超過 1,000 條直接拒絕，不做逐筆重放", async () => {
    const create = vi.spyOn(api, "memoryCreate").mockResolvedValue({});
    render(<BackupSection />);
    const items = Array.from({ length: MAX_BACKUP_ITEMS + 1 }, (_, i) => ({
      layer: "user-memory",
      kind: "preference",
      title: `t${i}`,
      content: "c",
    }));
    await userEvent.upload(screen.getByLabelText("選擇記憶備份檔"), backupFile(items));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/1,000 條上限/);
    expect(create).not.toHaveBeenCalled();
  });

  it("中途失敗：照實說已經寫進去幾筆，不假裝整批原子性", async () => {
    const create = vi
      .spyOn(api, "memoryCreate")
      .mockResolvedValueOnce({})
      .mockRejectedValueOnce(new Error("runtime 拒收"));
    render(<BackupSection />);
    const items = [
      { layer: "user-memory", kind: "preference", title: "一", content: "c" },
      { layer: "user-memory", kind: "preference", title: "二", content: "c" },
      { layer: "user-memory", kind: "preference", title: "三", content: "c" },
    ];
    await userEvent.upload(screen.getByLabelText("選擇記憶備份檔"), backupFile(items));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/runtime 拒收/);
    expect(alert).toHaveTextContent(/已成功寫入的 1 條會保留/);
    await waitFor(() => expect(create).toHaveBeenCalledTimes(2));
  });
});
