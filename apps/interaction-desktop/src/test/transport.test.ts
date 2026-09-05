import { afterEach, describe, expect, it, vi } from "vitest";
import { call, configureHttp } from "../transport";

describe("HTTP transport errors", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("surfaces the API's nested machine-readable error message", async () => {
    configureHttp("http://127.0.0.1:8787", "test-token");
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response(
          JSON.stringify({ error: { code: "policy_blocked", message: "需要確認" } }),
          { status: 403, statusText: "Forbidden" },
        ),
      ),
    );

    await expect(call("status")).rejects.toThrow("403: 需要確認");
  });

  // 「重新啟用裝置」是人類層動作，兩種傳輸走同一條 application service：
  // Tauri 內嵌模式是同名 IPC 指令，外部 daemon 模式是這條 HTTP 路由。
  it("provider_transition 打到 provider 的 transition 路由，狀態放在 body", async () => {
    configureHttp("http://127.0.0.1:8787", "test-token");
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ state: "available" })));
    vi.stubGlobal("fetch", fetchMock);

    await call("provider_transition", { id: "provider.adapter.esp32 desk", state: "available" });

    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    // id 一律經過編碼（空白／斜線不得直接拼進路徑）。
    expect(url).toBe(
      "http://127.0.0.1:8787/v1/providers/provider.adapter.esp32%20desk/transition"
    );
    expect(init.method).toBe("POST");
    expect(JSON.parse(String(init.body))).toEqual({ state: "available" });
  });
});
