import { describe, expect, it } from "vitest";
import { boundSemanticMemory, clampContextSize, parseCodexEntries, shouldExtractStoryState } from "../generate";

describe("Story State extraction schedule", () => {
  it("extracts initial turns and every third checkpoint", () => {
    expect(shouldExtractStoryState(1, "I wait.", "Rain falls.")).toBe(true);
    expect(shouldExtractStoryState(2, "I wait.", "Rain falls.")).toBe(true);
    expect(shouldExtractStoryState(3, "I wait.", "Rain falls.")).toBe(true);
    expect(shouldExtractStoryState(4, "I wait.", "Rain falls.")).toBe(false);
    expect(shouldExtractStoryState(6, "I wait.", "Rain falls.")).toBe(true);
  });

  it("extracts important state-changing events between checkpoints", () => {
    expect(shouldExtractStoryState(4, "I take the brass key.", "The key feels cold.")).toBe(true);
    expect(shouldExtractStoryState(5, "I wait.", "Mara reveals the hidden passage.")).toBe(true);
  });

  it("adjusts checkpoint frequency by performance profile", () => {
    expect(shouldExtractStoryState(3, "I wait.", "Rain falls.", "efficient")).toBe(false);
    expect(shouldExtractStoryState(4, "I wait.", "Rain falls.", "efficient")).toBe(true);
    expect(shouldExtractStoryState(5, "I wait.", "Rain falls.", "quality")).toBe(true);
  });
});

describe("context sizing", () => {
  it("caps context according to unified memory", () => {
    expect(clampContextSize("131072", 16)).toBe(8192);
    expect(clampContextSize("131072", 24)).toBe(16_384);
    expect(clampContextSize("131072", 64)).toBe(32_768);
  });
});

describe("semantic memory context", () => {
  it("bounds recalled scenes to their allotted input budget", () => {
    const memory = boundSemanticMemory([
      { message_id: "one", score: 0.9, content: "A".repeat(400) },
      { message_id: "two", score: 0.8, content: "B".repeat(400) },
    ], 50);
    expect(memory).toBeDefined();
    expect(memory!.length).toBeLessThanOrEqual(200);
    expect(memory).toContain("[0.90]");
  });
});

describe("Codex parsing", () => {
  it("skips only entries with malformed triggers", () => {
    const entries = parseCodexEntries([
      { id: "good", world_id: "w", title: "Good", content: "Lore", triggers: '["gate"]' },
      { id: "bad-json", world_id: "w", title: "Bad", content: "Lore", triggers: "{" },
      { id: "bad-shape", world_id: "w", title: "Bad", content: "Lore", triggers: '["gate", 4]' },
    ]);
    expect(entries).toHaveLength(1);
    expect(entries[0].id).toBe("good");
  });
});
