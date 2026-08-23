import type { StoryState, VisualBible } from "@/lib/tauri";

export type SceneImageStyle =
  | "anime"
  | "realistic"
  | "watercolor"
  | "ink"
  | "cinematic"
  | "dark-fantasy"
  | "manga";

export function buildSceneImagePrompt(
  scene: string,
  worldName: string,
  genre: string,
  state: StoryState | null,
  _style: SceneImageStyle,
  visualBible?: VisualBible | null,
): string {
  const stateContext = state
    ? [
        state.location && `Location: ${state.location}`,
        state.facts.length > 0 && `Facts: ${state.facts.slice(-4).join("; ")}`,
        state.relationships.length > 0 && `Visible relationships: ${state.relationships.slice(-3).join("; ")}`,
        state.inventory.length > 0 && `Important props: ${state.inventory.slice(-5).join(", ")}`,
        state.active_goals.length > 0 && `Active goals: ${state.active_goals.slice(-3).join("; ")}`,
        state.open_threads.length > 0 && `Open threads: ${state.open_threads.slice(-3).join("; ")}`,
        state.quests?.length && `Quests: ${state.quests.slice(-3).map((quest) => `${quest.title} (${quest.status})`).join("; ")}`,
        state.inventory_items?.length && `Structured inventory: ${state.inventory_items.slice(-5).map((item) => `${item.name} x${item.quantity} (${item.condition})`).join("; ")}`,
        state.relationship_records?.length && `Structured relationships: ${state.relationship_records.slice(-3).map((record) => `${record.character}: ${record.status}`).join("; ")}`,
        state.conflicts.length > 0 && `Continuity warnings: ${state.conflicts.slice(-2).join("; ")}`,
        state.last_scene && `Previous visual beat: ${state.last_scene}`,
      ].filter(Boolean).join("\n")
    : "";
  const sceneContext = scene.length > 1400
    ? `${scene.slice(0, 700)} ... ${scene.slice(-700)}`
    : scene;

  return [
    `Story: ${worldName} (${genre || "fantasy"})`,
    visualBible && [
      visualBible.style && `Visual style: ${visualBible.style}`,
      visualBible.palette && `Color palette: ${visualBible.palette}`,
      visualBible.character_anchors && `Character continuity: ${visualBible.character_anchors}`,
      visualBible.location_anchors && `Location continuity: ${visualBible.location_anchors}`,
      visualBible.negative_prompt && `Avoid: ${visualBible.negative_prompt}`,
    ].filter(Boolean).join("\n"),
    stateContext,
    `Current visual beat: ${sceneContext}`,
    "One coherent moment, no collage, no written text, no subtitles, no watermark, no interface, no random modern objects.",
    "Keep the setting, props, lighting, and named characters consistent with the story. Do not depict an unseen decision by the player.",
  ].filter(Boolean).join("\n\n");
}
