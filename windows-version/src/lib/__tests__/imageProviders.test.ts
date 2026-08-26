import { describe, expect, it } from "vitest";
import { CLOUD_IMAGE_PROVIDERS } from "@/lib/imageProviders";

describe("CLOUD_IMAGE_PROVIDERS registry", () => {
  it("lists exactly the 7 cloud image providers in UI order, ids unique", () => {
    const ids = CLOUD_IMAGE_PROVIDERS.map((e) => e.id);
    expect(ids).toEqual(["openai", "seedream", "hunyuan", "cogview", "flux", "fal", "imagen"]);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("maps every provider to a non-empty BYOK key slot", () => {
    for (const e of CLOUD_IMAGE_PROVIDERS) expect(e.byokKey.length).toBeGreaterThan(0);
  });

  it("carries i18n keys with English fallbacks", () => {
    for (const e of CLOUD_IMAGE_PROVIDERS) {
      expect(e.labelKey.length).toBeGreaterThan(0);
      expect(e.descKey.length).toBeGreaterThan(0);
      expect(e.setupKey.length).toBeGreaterThan(0);
      expect(e.labelFallback.length).toBeGreaterThan(0);
      expect(e.descFallback.length).toBeGreaterThan(0);
      expect(e.setupFallback.length).toBeGreaterThan(0);
    }
  });

  it("keeps the fal entry wired to its BYOK slot and i18n keys", () => {
    const fal = CLOUD_IMAGE_PROVIDERS.find((e) => e.id === "fal")!;
    expect(fal.byokKey).toBe("fal");
    expect(fal.labelKey).toBe("ui.fal-ai");
    expect(fal.descKey).toBe("ui.fal-desc");
  });
});
