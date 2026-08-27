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
});
