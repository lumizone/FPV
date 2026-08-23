import { describe, expect, it } from "vitest";
import { buildSceneImagePrompt } from "../imagePrompt";

describe("buildSceneImagePrompt", () => {
  it("combines scene and structured continuity context", () => {
    const prompt = buildSceneImagePrompt(
      "Mara holds the brass key beneath the station clock.",
      "The Last Station",
      "mystery",
      {
        turn: 3,
        location: "Old station",
        active_goals: ["Open the north gate"],
        open_threads: ["Who left the key?"],
        inventory: ["brass key"],
        relationships: ["Mara: cautious ally"],
        facts: ["The gate opens at midnight"],
        conflicts: [],
        last_scene: "Mara arrived before dawn.",
        source: "model",
        quests: [{ id: "gate", title: "Open the gate", status: "active", description: "Find the key" }],
        inventory_items: [{ name: "brass key", quantity: 1, condition: "intact" }],
        relationship_records: [{ character: "Mara", status: "ally", note: "" }],
      },
      "cinematic",
    );

    expect(prompt).toContain("Old station");
    expect(prompt).toContain("brass key");
    expect(prompt).toContain("Open the north gate");
    expect(prompt).toContain("Who left the key?");
    expect(prompt).toContain("Previous visual beat");
    expect(prompt).toContain("no watermark");
  });

  it("adds visual bible continuity instructions", () => {
    const prompt = buildSceneImagePrompt(
      "Mara enters the observatory.", "The Last Station", "mystery", null, "cinematic",
      {
        style: "cinematic anime",
        palette: "indigo shadows and amber light",
        character_anchors: "Mara: short black hair, red scarf",
        location_anchors: "Old observatory with a cracked glass dome",
        negative_prompt: "modern cars, text, watermark",
      },
    );
    expect(prompt).toContain("cinematic anime");
    expect(prompt).toContain("Mara: short black hair");
    expect(prompt).toContain("cracked glass dome");
    expect(prompt).toContain("modern cars");
  });
});
