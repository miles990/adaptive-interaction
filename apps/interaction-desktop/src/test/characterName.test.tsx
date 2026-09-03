// 角色名稱的單一真相（src/characterName.ts）：
//   優先序 prefs.companionName ＞ manifest displayName（依 locale）＞「角色」；
//   代詞走 manifest.pronouns，缺省中立；載入失敗不猜、顯示「角色」；
//   Runtime 的 activeCharacter 有值就讀 manifest，讀不到退回 activeCharacter；
//   否則用 bundled 索引（prefs.companionPack 命中，否則 default）；
//   導覽第二項（simpleNavFor）、標題（titleFor）、全域搜尋（pagesFor）跟著名稱走。

import fs from "node:fs";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { api } from "../api";
import { SIMPLE_NAV, simpleNavFor, titleFor } from "../App";
import { pagesFor } from "../components/GlobalSearch";
import {
  characterNameFallback,
  CHARACTER_NAME_MIN_REFRESH_MS,
  currentCharacterName,
  NEUTRAL_CHARACTER_ICON,
  neutralPronoun,
  pickBundledManifest,
  primeCharacterNameForTests,
  refreshCharacterName,
  resetCharacterNameForTests,
  resolveCharacterName,
  useCharacterName,
} from "../characterName";
import type { CharacterIndex } from "../character/registry";
import { displayNameOf } from "../character/manifest";

const SHU = {
  characterId: "shu-maid",
  displayName: { "zh-TW": "小樞", en: "Shu" },
  pronouns: { "zh-TW": "她", en: "she" },
};

const STATUS_WITH_SHU = {
  characterProtocol: {
    version: "1.0",
    instances: 1,
    activeCharacter: { characterId: "shu-maid", displayName: SHU.displayName },
  },
};

const failingFetch = () =>
  vi.fn(async () => {
    throw new Error("offline");
  });

beforeEach(() => {
  resetCharacterNameForTests();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function Probe() {
  const c = useCharacterName();
  return (
    <div>
      <span data-testid="name">{c.name}</span>
      <span data-testid="pronoun">{c.pronoun}</span>
      <span data-testid="id">{String(c.characterId)}</span>
      <span data-testid="loaded">{String(c.loaded)}</span>
      <span data-testid="icon">{c.icon}</span>
      <nav aria-label="probe-nav">
        {simpleNavFor(c).map((t) => (
          <span key={t.id} data-testid={`nav-${t.id}`}>
            {t.label}
          </span>
        ))}
      </nav>
    </div>
  );
}

// ---------------------------------------------------------------------------

describe("resolveCharacterName：優先序與 fallback（純函式）", () => {
  it("使用者取的名字優先於 manifest displayName", () => {
    const r = resolveCharacterName({ companionName: "  阿花  " }, SHU, "zh-TW");
    expect(r.name).toBe("阿花");
    expect(r.characterId).toBe("shu-maid");
    expect(r.loaded).toBe(true);
  });

  it("沒取名字就用 manifest 的 displayName（依 locale），代詞來自 manifest", () => {
    expect(resolveCharacterName(null, SHU, "zh-TW")).toMatchObject({
      name: "小樞",
      pronoun: "她",
      characterId: "shu-maid",
      loaded: true,
    });
    expect(resolveCharacterName({ companionName: "" }, SHU, "en")).toMatchObject({
      name: "Shu",
      pronoun: "she",
    });
  });

  it("沒有 manifest：名字是中立的「角色」、代詞中立、loaded=false", () => {
    const zh = resolveCharacterName(null, null, "zh-TW");
    expect(zh).toEqual({
      name: characterNameFallback,
      pronoun: "角色",
      characterId: null,
      loaded: false,
      icon: NEUTRAL_CHARACTER_ICON,
    });
    expect(resolveCharacterName(null, null, "en").pronoun).toBe("they");
    expect(neutralPronoun("zh-TW")).toBe("角色");
    expect(neutralPronoun("en-US")).toBe("they");
  });

  it("manifest 沒宣告代詞時用中立文案（不借別的語言的代詞）", () => {
    const noPronoun = { characterId: "plain-text", displayName: { "zh-TW": "文字角色" } };
    expect(resolveCharacterName(null, noPronoun, "zh-TW").pronoun).toBe("角色");
    expect(
      resolveCharacterName(null, { ...SHU, pronouns: { en: "she" } }, "zh-TW").pronoun
    ).toBe("角色");
  });

  it("取的名字只保留前 24 字；純空白視為沒取名字", () => {
    const long = "名".repeat(40);
    expect(resolveCharacterName({ companionName: long }, SHU).name).toBe("名".repeat(24));
    expect(resolveCharacterName({ companionName: "   " }, SHU).name).toBe("小樞");
    expect(resolveCharacterName({ companionName: "   " }, null).name).toBe("角色");
  });

  it("icon：manifest 提示命中目錄才採用，否則中立 icon（不讓導覽出現問號）", () => {
    expect(resolveCharacterName(null, { ...SHU, icon: "cat" }).icon).toBe("cat");
    expect(resolveCharacterName(null, { ...SHU, icon: "no-such-icon" }).icon).toBe(
      NEUTRAL_CHARACTER_ICON
    );
    expect(resolveCharacterName(null, { ...SHU, icon: 42 }).icon).toBe(NEUTRAL_CHARACTER_ICON);
    expect(resolveCharacterName(null, SHU).icon).toBe(NEUTRAL_CHARACTER_ICON);
  });
});

describe("pickBundledManifest：prefs.companionPack 命中就用它，否則索引 default", () => {
  const index = {
    schemaVersion: "1.0",
    default: "shu-maid",
    characters: [
      { characterId: "shu-maid", manifest: SHU },
      { characterId: "plain-text", manifest: { characterId: "plain-text", displayName: { "zh-TW": "文字角色" } } },
    ],
    errors: [],
  } as unknown as CharacterIndex;

  it("命中偏好", () => {
    expect(pickBundledManifest(index, "plain-text")?.characterId).toBe("plain-text");
  });
  it("偏好不在索引裡（或沒偏好）→ default", () => {
    expect(pickBundledManifest(index, "not-there")?.characterId).toBe("shu-maid");
    expect(pickBundledManifest(index, null)?.characterId).toBe("shu-maid");
    expect(pickBundledManifest(index, "")?.characterId).toBe("shu-maid");
  });
  it("連 default 都不在 → null（不編造角色）", () => {
    expect(pickBundledManifest({ ...index, default: "ghost" }, undefined)).toBeNull();
  });
});

// ---------------------------------------------------------------------------

describe("useCharacterName：Runtime 有 activeCharacter 就讀 manifest", () => {
  it("manifest 讀得到：名字／代詞／id 來自 manifest，導覽第二項就是名字", async () => {
    vi.stubGlobal("fetch", failingFetch());
    vi.spyOn(api, "status").mockResolvedValue(STATUS_WITH_SHU);
    const manifest = vi.spyOn(api, "characterManifest").mockResolvedValue(SHU as never);
    render(<Probe />);
    expect(await screen.findByText("小樞", { selector: "[data-testid=name]" })).toBeInTheDocument();
    expect(screen.getByTestId("pronoun")).toHaveTextContent("她");
    expect(screen.getByTestId("id")).toHaveTextContent("shu-maid");
    expect(screen.getByTestId("loaded")).toHaveTextContent("true");
    expect(screen.getByTestId("nav-companion")).toHaveTextContent("小樞");
    expect(manifest).toHaveBeenCalledTimes(1);
  });

  it("manifest 讀不到（尚未 hello／舊 daemon）：退回 Runtime 回報的 activeCharacter，代詞中立", async () => {
    vi.stubGlobal("fetch", failingFetch());
    vi.spyOn(api, "status").mockResolvedValue(STATUS_WITH_SHU);
    vi.spyOn(api, "characterManifest").mockRejectedValue(new Error("404 Not Found"));
    render(<Probe />);
    expect(await screen.findByText("小樞", { selector: "[data-testid=name]" })).toBeInTheDocument();
    expect(screen.getByTestId("pronoun")).toHaveTextContent("角色");
    expect(screen.getByTestId("loaded")).toHaveTextContent("true");
  });

  it("什麼都讀不到：顯示「角色」、loaded=false，導覽第二項也是「角色」", async () => {
    vi.stubGlobal("fetch", failingFetch());
    vi.spyOn(api, "status").mockRejectedValue(new Error("offline"));
    vi.spyOn(api, "characterManifest").mockRejectedValue(new Error("offline"));
    render(<Probe />);
    await waitFor(() => expect(screen.getByTestId("loaded")).toHaveTextContent("false"));
    expect(screen.getByTestId("name")).toHaveTextContent(characterNameFallback);
    expect(screen.getByTestId("nav-companion")).toHaveTextContent(characterNameFallback);
    expect(screen.getByTestId("id")).toHaveTextContent("null");
  });

  it("Runtime 沒有 activeCharacter：用 bundled 索引的 default（真實 index／manifest 檔）", async () => {
    const indexJson = JSON.parse(
      fs.readFileSync(path.resolve("public/characters/index.json"), "utf8")
    ) as { characters: { characterId: string; manifestPath: string }[] };
    const plain = indexJson.characters.find((c) => c.characterId === "plain-text");
    expect(plain).toBeTruthy();
    const manifestText = fs.readFileSync(path.resolve("public/characters/plain-text/manifest.json"), "utf8");
    const served: Record<string, string> = {
      "/characters/index.json": JSON.stringify({
        schemaVersion: "1.0",
        default: "plain-text",
        characters: [plain],
      }),
      [plain!.manifestPath]: manifestText,
    };
    const fetchMock = vi.fn(async (url: string) => {
      const body = served[url];
      return { ok: body !== undefined, status: body !== undefined ? 200 : 404, text: async () => body ?? "" };
    });
    vi.stubGlobal("fetch", fetchMock);
    vi.spyOn(api, "status").mockResolvedValue({ characterProtocol: { version: "1.0", instances: 0, activeCharacter: null } });
    const manifest = vi.spyOn(api, "characterManifest").mockRejectedValue(new Error("404"));
    render(<Probe />);
    const expected = displayNameOf(JSON.parse(manifestText), "zh-TW");
    expect(await screen.findByText(expected, { selector: "[data-testid=name]" })).toBeInTheDocument();
    expect(screen.getByTestId("id")).toHaveTextContent("plain-text");
    expect(screen.getByTestId("loaded")).toHaveTextContent("true");
    // 沒有 activeCharacter 就不去問 manifest；只讀同源索引與 manifest 檔。
    expect(manifest).not.toHaveBeenCalled();
    for (const [url] of fetchMock.mock.calls) expect(url.startsWith("/characters/")).toBe(true);
  });

  it("多個元件同時掛載只打一輪 API；短時間內再刷新不重打，force 才重打", async () => {
    vi.stubGlobal("fetch", failingFetch());
    const status = vi.spyOn(api, "status").mockResolvedValue(STATUS_WITH_SHU);
    vi.spyOn(api, "characterManifest").mockResolvedValue(SHU as never);
    render(
      <>
        <Probe />
        <Probe />
      </>
    );
    await waitFor(() => expect(screen.getAllByTestId("loaded").map((e) => e.textContent)).toEqual(["true", "true"]));
    expect(status).toHaveBeenCalledTimes(1);
    await refreshCharacterName();
    expect(status).toHaveBeenCalledTimes(1);
    await refreshCharacterName({ force: true });
    expect(status).toHaveBeenCalledTimes(2);
    expect(CHARACTER_NAME_MIN_REFRESH_MS).toBeGreaterThan(0);
    expect(currentCharacterName().name).toBe("小樞");
  });

  it("角色換了（force 刷新）：所有訂閱者一起更新", async () => {
    vi.stubGlobal("fetch", failingFetch());
    const status = vi.spyOn(api, "status").mockResolvedValue(STATUS_WITH_SHU);
    const manifest = vi.spyOn(api, "characterManifest").mockResolvedValue(SHU as never);
    render(<Probe />);
    await screen.findByText("小樞", { selector: "[data-testid=name]" });
    status.mockResolvedValue({
      characterProtocol: {
        version: "1.0",
        instances: 1,
        activeCharacter: { characterId: "plain-text", displayName: { "zh-TW": "文字角色" } },
      },
    });
    manifest.mockResolvedValue({ characterId: "plain-text", displayName: { "zh-TW": "文字角色" } } as never);
    await refreshCharacterName({ force: true });
    await screen.findByText("文字角色", { selector: "[data-testid=name]" });
    expect(screen.getByTestId("nav-companion")).toHaveTextContent("文字角色");
    expect(screen.getByTestId("pronoun")).toHaveTextContent("角色");
  });
});

describe("primeCharacterNameForTests：其他測試可直接釘住角色，不必 mock 三個 API", () => {
  it("釘住後掛載的一般刷新不會蓋掉；force 刷新才重新解析", async () => {
    vi.stubGlobal("fetch", failingFetch());
    const status = vi.spyOn(api, "status").mockRejectedValue(new Error("offline"));
    primeCharacterNameForTests({ name: "小樞", pronoun: "她", characterId: "shu-maid" });
    render(<Probe />);
    expect(screen.getByTestId("name")).toHaveTextContent("小樞");
    expect(screen.getByTestId("pronoun")).toHaveTextContent("她");
    expect(screen.getByTestId("loaded")).toHaveTextContent("true");
    await Promise.resolve();
    expect(status).not.toHaveBeenCalled();
    expect(screen.getByTestId("nav-companion")).toHaveTextContent("小樞");
    await refreshCharacterName({ force: true });
    expect(status).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.getByTestId("name")).toHaveTextContent(characterNameFallback));
  });

  it("reset 解除釘住", async () => {
    vi.stubGlobal("fetch", failingFetch());
    primeCharacterNameForTests({ name: "阿花" });
    resetCharacterNameForTests();
    expect(currentCharacterName().name).toBe(characterNameFallback);
    const status = vi.spyOn(api, "status").mockRejectedValue(new Error("offline"));
    await refreshCharacterName();
    expect(status).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------------------

describe("導覽／標題／全域搜尋的角色名稱來源", () => {
  it("SIMPLE_NAV 靜態表第二項是中立值；simpleNavFor 只換第二項的 label 與 icon，仍恰 5 項", () => {
    expect(SIMPLE_NAV[1]).toEqual({ id: "companion", label: characterNameFallback, icon: NEUTRAL_CHARACTER_ICON });
    const nav = simpleNavFor({ name: "阿花", icon: "cat" });
    expect(nav).toHaveLength(5);
    expect(nav.map((t) => t.id)).toEqual(SIMPLE_NAV.map((t) => t.id));
    expect(nav[1]).toEqual({ id: "companion", label: "阿花", icon: "cat" });
    expect(nav.filter((t) => t.id !== "companion")).toEqual(SIMPLE_NAV.filter((t) => t.id !== "companion"));
  });

  it("titleFor：角色頁標題是傳入的名字；沒傳就是「角色」；其他頁不受影響", () => {
    expect(titleFor("companion", "阿花")).toBe("阿花");
    expect(titleFor("companion")).toBe(characterNameFallback);
    expect(titleFor("home", "阿花")).toBe("現在");
    expect(titleFor("ai", "阿花")).toBe("工作");
    expect(titleFor("manage", "阿花")).toBe("更多");
    expect(titleFor("advanced-features")).toBe("更多");
  });

  it("pagesFor：全域搜尋的角色頁 label 跟著名字，其他頁面不變且包含新分頁", () => {
    const pages = pagesFor("阿花");
    expect(pages.find((p) => p.id === "companion")?.label).toBe("阿花");
    expect(pages.find((p) => p.id === "home")?.label).toBe("現在");
    expect(pages.map((p) => p.id)).toEqual(expect.arrayContaining(["manage", "advanced-features"]));
  });
});
