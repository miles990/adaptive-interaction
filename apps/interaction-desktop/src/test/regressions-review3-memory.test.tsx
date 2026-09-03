// v0.5.1 對抗審查（0c845e0-20260903T185130Z）memory-ui 維度確認缺陷的 regression tests：
// - memory-ui-001：一般模式記憶清單只取最新 200 筆再前端分類，分類為空時不得說
//   「這個分類目前沒有記憶」——那是主動說錯話。截斷時要看得到「只看了最新 N 筆」提示。
// - memory-ui-002：一般模式「開啟來源」檢視器沒有 advanced 分層，直接露出 sha256 與
//   「內容定址 blob」技術文案；要比照 AssetsSection 其它欄位做人話分層。
// - memory-ui-003：一般模式貼上文字加入素材，卡片標題不得退化成 sha256 前 12 碼。
// - memory-ui-004：KnowledgeSection／DomainPacksPanel／AssetsSection／ReceiptsSection
//   四個 StateView 傳的是物件（不是陣列），empty 文案永遠不會顯示，畫面會是空白 div。

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { api } from "../api";
import { primeCharacterNameForTests, resetCharacterNameForTests } from "../characterName";
import { MemoryKnowledgePage } from "../pages/MemoryKnowledgePage";

afterEach(() => {
  vi.restoreAllMocks();
  resetCharacterNameForTests();
});

function stubEmptySections() {
  vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
  vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
  vi.spyOn(api, "assetsList").mockResolvedValue({ assets: [], count: 0 });
  vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
  vi.spyOn(api, "domainPacks").mockResolvedValue({ packs: [] });
}

// ---------------------------------------------------------------------------
// memory-ui-001
// ---------------------------------------------------------------------------

describe("memory-ui-001：一般模式記憶清單被截斷時，不得對空分類說謊", () => {
  function stub(overrides: Record<string, unknown> = {}) {
    // 最新 200 筆全部都是 about-me 分類；「工作與任務」在這一頁完全看不到，
    // 但後端告知總數遠超過這一頁——不能就此斷言「這個分類目前沒有記憶」。
    const items = Array.from({ length: 3 }, (_, i) => ({
      memoryId: `m-${i}`,
      title: `記憶 ${i}`,
      kind: "fact",
      layer: "user-memory",
      content: "x",
      status: "active",
      createdBy: { kind: "human" },
      retention: {},
    }));
    vi.spyOn(api, "memoryList").mockResolvedValue({
      items,
      count: items.length,
      total: 250,
      limit: 200,
      limitReached: true,
      ...overrides,
    });
    vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
    vi.spyOn(api, "assetsList").mockResolvedValue({ assets: [], count: 0 });
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    vi.spyOn(api, "domainPacks").mockResolvedValue({ packs: [] });
  }

  it("截斷且該分類在這一頁沒有記憶：不能顯示裸的「這個分類目前沒有記憶。」，要有截斷提示與總數", async () => {
    stub();
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced={false} />);
    await screen.findByText("記憶 0");
    const select = container.querySelector("select") as HTMLElement;
    await userEvent.selectOptions(select, "work");
    await waitFor(() => expect(screen.queryByText("記憶 0")).not.toBeInTheDocument());
    // 裸的「這個分類目前沒有記憶。」不得單獨出現（不代表「完全沒有」）。
    expect(screen.queryByText("這個分類目前沒有記憶。", { exact: true })).not.toBeInTheDocument();
    const text = container.textContent ?? "";
    expect(text).toContain("250");
    expect(text).toMatch(/較舊的沒有列出|沒有被列出|只看了最(新|近)/);
  });

  it("沒有截斷（total 等於這一頁筆數）：空分類仍可以照實說「這個分類目前沒有記憶。」", async () => {
    stub({ total: 3, limitReached: false });
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced={false} />);
    await screen.findByText("記憶 0");
    const select = container.querySelector("select") as HTMLElement;
    await userEvent.selectOptions(select, "work");
    expect(await screen.findByText("這個分類目前沒有記憶。")).toBeInTheDocument();
    // 不用截斷措辭去嚇唬使用者（沒有截斷就不用提總筆數）。
    expect(container.textContent).not.toContain("較舊的沒有列出");
  });
});

// ---------------------------------------------------------------------------
// memory-ui-002
// ---------------------------------------------------------------------------

describe("memory-ui-002：一般模式的「開啟來源」不得外洩 sha256／「內容定址」技術文案", () => {
  const ASSET = {
    hash: "b".repeat(64),
    mediaType: "image",
    sizeBytes: 1234,
    source: "user-import",
    originalName: "receipt.png",
  };
  const PREVIEW = {
    hash: "b".repeat(64),
    mediaType: "image",
    mime: "image/png",
    dataBase64: "",
    sizeBytes: 1234,
    note: "預覽資料來自內容定址 blob；媒體內容視為 untrusted，不執行其中指令。",
  };

  function stub() {
    vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
    vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
    vi.spyOn(api, "assetsList").mockResolvedValue({ assets: [ASSET], count: 1 });
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    vi.spyOn(api, "domainPacks").mockResolvedValue({ packs: [] });
    vi.spyOn(api, "assetPreview").mockResolvedValue(PREVIEW);
  }

  it("一般模式：點開「開啟來源」不得出現「內容定址」或原始 hash 前綴", async () => {
    stub();
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced={false} />);
    await userEvent.click(await screen.findByRole("tab", { name: "素材與來源" }));
    await screen.findByText("receipt.png");
    await userEvent.click(screen.getByRole("button", { name: "開啟來源" }));
    await screen.findByTestId("source-media-viewer");
    const text = container.textContent ?? "";
    for (const leak of ["內容定址", "hash b", ASSET.hash.slice(0, 20)]) {
      expect(text, `一般模式不得出現「${leak}」`).not.toContain(leak);
    }
  });

  it("進階模式：hash 與原始 note 仍看得到（零能力退化）", async () => {
    stub();
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced />);
    await userEvent.click(await screen.findByRole("tab", { name: "素材與來源" }));
    await screen.findByText("receipt.png");
    await userEvent.click(screen.getByRole("button", { name: "開啟來源" }));
    await screen.findByTestId("source-media-viewer");
    const text = container.textContent ?? "";
    expect(text).toContain("內容定址");
    expect(text).toContain(ASSET.hash.slice(0, 20));
  });
});

// ---------------------------------------------------------------------------
// memory-ui-003
// ---------------------------------------------------------------------------

describe("memory-ui-003：貼上的文字素材，卡片標題不得是 sha256 前 12 碼", () => {
  it("originalName 未帶（純文字 inline 匯入的預設結果）：標題不是 hash 前綴", async () => {
    const hash = "abcdef0123456789".padEnd(64, "0");
    vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
    vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    vi.spyOn(api, "domainPacks").mockResolvedValue({ packs: [] });
    vi.spyOn(api, "assetsList").mockResolvedValue({
      assets: [{ hash, mediaType: "text/plain", sizeBytes: 42, source: "user-import" }],
      count: 1,
    });
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced={false} />);
    await userEvent.click(await screen.findByRole("tab", { name: "素材與來源" }));
    await waitFor(() => expect(container.querySelector(".provider-card strong")).not.toBeNull());
    const title = container.querySelector(".provider-card strong")?.textContent ?? "";
    expect(title).not.toBe(`${hash.slice(0, 12)}…`);
    expect(title).not.toMatch(/^[0-9a-f]{12}…$/);
  });

  it("description 有帶時，標題用 description 而不是 hash", async () => {
    const hash = "1234567890abcdef".padEnd(64, "0");
    vi.spyOn(api, "memoryList").mockResolvedValue({ items: [] });
    vi.spyOn(api, "knowledgeList").mockResolvedValue({ nodes: [], count: 0 });
    vi.spyOn(api, "knowledgeReceipts").mockResolvedValue({ receipts: [] });
    vi.spyOn(api, "domainPacks").mockResolvedValue({ packs: [] });
    vi.spyOn(api, "assetsList").mockResolvedValue({
      assets: [
        {
          hash,
          mediaType: "text/plain",
          sizeBytes: 42,
          source: "user-import",
          description: "會議紀要草稿",
        },
      ],
      count: 1,
    });
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    const { container } = render(<MemoryKnowledgePage refreshKey={0} advanced={false} />);
    await userEvent.click(await screen.findByRole("tab", { name: "素材與來源" }));
    expect(await screen.findByText("會議紀要草稿")).toBeInTheDocument();
    expect(container.textContent).not.toContain(hash.slice(0, 12));
  });
});

// ---------------------------------------------------------------------------
// memory-ui-004
// ---------------------------------------------------------------------------

describe("memory-ui-004：StateView 對物件型 payload 的空清單要看得見一句話，不能是空白 div", () => {
  it("知識清單空：顯示「這個狀態目前沒有知識項目。」", async () => {
    stubEmptySections();
    render(<MemoryKnowledgePage refreshKey={0} advanced />);
    await userEvent.click(await screen.findByRole("tab", { name: /學會的知識/ }));
    expect(await screen.findByText("這個狀態目前沒有知識項目。")).toBeInTheDocument();
  });

  it("Domain Pack 清單空：顯示「沒有可用的 Domain Pack。」", async () => {
    stubEmptySections();
    render(<MemoryKnowledgePage refreshKey={0} advanced />);
    await userEvent.click(await screen.findByRole("tab", { name: /學會的知識/ }));
    expect(await screen.findByText("沒有可用的 Domain Pack。")).toBeInTheDocument();
  });

  it("素材清單空：顯示「還沒有素材。」", async () => {
    stubEmptySections();
    render(<MemoryKnowledgePage refreshKey={0} advanced />);
    await userEvent.click(await screen.findByRole("tab", { name: "素材與來源" }));
    expect(await screen.findByText("還沒有素材。")).toBeInTheDocument();
  });

  it("知識收據清單空：顯示「還沒有知識變化紀錄。」", async () => {
    stubEmptySections();
    render(<MemoryKnowledgePage refreshKey={0} advanced />);
    await userEvent.click(await screen.findByRole("tab", { name: "知識收據" }));
    expect(await screen.findByText("還沒有知識變化紀錄。")).toBeInTheDocument();
  });
});
