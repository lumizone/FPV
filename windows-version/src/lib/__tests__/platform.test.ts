import { describe, it, expect, vi } from "vitest";

describe("platform detection", () => {
  it("detects Windows from a Windows UA string", async () => {
    vi.stubGlobal("navigator", { userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)" });
    vi.resetModules();
    const { isWindows, isMac, modKey } = await import("../platform");
    expect(isWindows).toBe(true);
    expect(isMac).toBe(false);
    expect(modKey).toBe("Ctrl+");
  });

  it("detects Mac from a Macintosh UA string", async () => {
    vi.stubGlobal("navigator", { userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)" });
    vi.resetModules();
    const { isMac, modKey } = await import("../platform");
    expect(isMac).toBe(true);
    expect(modKey).toBe("⌘");
  });
});
