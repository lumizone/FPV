/**
 * End-to-end narration generation — wires the full pipeline:
 *   Tauri IPC (world + codex + message loading) → promptBuilder →
 *   Tauri IPC (Ollama generation) → orchestrator (post-processing) →
 *   Tauri IPC (message persistence).
 *
 * Imported by UI components (Task 24+) that need a single
 * `generateNarration(sessionId, worldId, userAction, summary?)` call.
 */

import { invoke } from "@tauri-apps/api/core";
import { useApp } from "@/lib/store";
import { estimateTokens, limitNarrationWords, matchCodex, postProcess, trimContext, type CodexEntry, type Message } from "./orchestrator";
import { buildPrompt, buildSystemFor, getNarrationBudget, OPENING_INSTRUCTION, sanitizeForPrompt, sanitizeStrictForPrompt } from "./promptBuilder";
import type { StoryState } from "@/lib/tauri";

// ---------------------------------------------------------------------------
// Types matching the Rust backend
// ---------------------------------------------------------------------------

interface World {
  id: string;
  name: string;
  genre: string;
  description: string | null;
  system_prompt: string;
  accent_color: string | null;
  is_nsfw: boolean;
  cover_image_path: string | null;
  source: string;
  privacy_mode: "local_only" | "cloud_allowed";
}

interface CodexEntryRaw {
  id: string;
  world_id: string;
  title: string;
  content: string;
  triggers: string; // JSON string from SQLite TEXT column
}

interface CharacterRaw {
  id: string;
  world_id: string;
  name: string;
  role: string;
  backstory: string | null;
  traits: string; // JSON array
  avatar_path: string | null;
  created_at: number;
  updated_at: number;
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Optional generation parameters for Ollama inference. */
export interface GenerationParams {
  temperature?: number;
  top_p?: number;
  max_tokens?: number;
}

const DEFAULT_CONTEXT_SIZE = 8192;
const MIN_CONTEXT_SIZE = 512;
const CONTEXT_SAFETY_RATIO = 0.12;
const continuityJobs = new Map<string, Promise<void>>();

export function boundSemanticMemory(
  scenes: { message_id: string; content: string; score: number }[],
  tokenBudget: number,
): string | undefined {
  let remaining = Math.max(0, tokenBudget);
  const lines: string[] = [];
  for (const scene of scenes) {
    const prefix = `- [${scene.score.toFixed(2)}] `;
    const maxChars = Math.max(0, remaining * 4 - prefix.length);
    if (maxChars === 0) break;
    const line = prefix + scene.content.slice(0, maxChars);
    const tokens = estimateTokens(line);
    if (tokens > remaining) break;
    lines.push(line);
    remaining -= tokens;
  }
  return lines.length > 0 ? lines.join("\n") : undefined;
}

export async function waitForContinuity(sessionId: string): Promise<void> {
  await continuityJobs.get(sessionId);
}

export function clampContextSize(value: string | undefined, ramGb = 16): number {
  const parsed = Number.parseInt(value ?? "", 10);
  const hardwareLimit = ramGb <= 16 ? 8192 : ramGb <= 32 ? 16_384 : 32_768;
  if (!Number.isFinite(parsed)) return Math.min(DEFAULT_CONTEXT_SIZE, hardwareLimit);
  return Math.min(hardwareLimit, Math.max(MIN_CONTEXT_SIZE, parsed));
}

function clampOutputTokens(value: number | undefined): number {
  return Math.min(4096, Math.max(64, value ?? 2048));
}

function capText(value: string, maxChars: number): string {
  if (value.length <= maxChars) return value;
  return `${value.slice(0, Math.max(0, maxChars - 32))}\n[content truncated for context budget]`;
}

export function shouldExtractStoryState(turn: number, action: string, narration: string, profile = "balanced"): boolean {
  if (profile === "quality") return true;
  const checkpoint = profile === "efficient" ? 4 : 3;
  if (turn <= 2 || turn % checkpoint === 0) return true;
  const event = `${action} ${narration}`.toLowerCase();
  return /\b(enter|leave|arrive|travel|move to|take|pick up|drop|give|receive|lose|meet|join|betray|kill|die|injur|reveal|discover|unlock|promise|agree|confess)\w*\b/.test(event);
}

export function parseCodexEntries(rawCodex: CodexEntryRaw[]): CodexEntry[] {
  return rawCodex.flatMap((c) => {
    try {
      const triggers = JSON.parse(c.triggers || "[]");
      if (!Array.isArray(triggers) || !triggers.every((trigger) => typeof trigger === "string")) {
        throw new Error("triggers must be an array of strings");
      }
      return [{ id: c.id, title: capText(c.title, 240), content: capText(c.content, 1800), triggers }];
    } catch (error) {
      console.warn(`Skipping malformed Codex triggers for entry ${c.id}:`, error);
      return [];
    }
  });
}

// ---------------------------------------------------------------------------
// generateNarration
// ---------------------------------------------------------------------------

/**
 * Run a full narration turn for a session in a given world:
 *
 * 1. Load world metadata, message history, and codex entries (parallel).
 * 2. Match codex triggers against recent context + the user action.
 * 3. Assemble the model prompt (system prompt + summary + matched lore + history + action).
 * 4. Generate via the local Ollama model.
 * 5. Post-process (strip markdown, clean verbal tics).
 * 6. Persist both the user action and narrator response to the session.
 * 7. Return the narrator's response for display.
 *
 * @param sessionId - The session to read history from and persist into.
 * @param worldId   - The world defining system prompt, genre, and lore.
 * @param userAction - The player's current action text.
 * @param summary   - Optional rolling session summary for context.
 * @param genParams - Optional generation parameter overrides (temperature, top_p, max_tokens).
 * @returns The generated narrator message content (post-processed).
 */
async function generateNarrationUnlocked(
  sessionId: string,
  worldId: string,
  userAction: string,
  summary?: string,
  genParams?: GenerationParams,
  preferences?: Record<string, string>,
  persistUserAction = true,
  replaceMessageId?: string,
  signal?: AbortSignal,
): Promise<{ content: string; memorySummary?: string; narratorId: string; userId?: string; storyState: StoryState; continuity: Promise<StoryState> }> {
  const throwIfAborted = () => {
    if (signal?.aborted) throw new Error("Narration cancelled");
  };
  throwIfAborted();
  // 1. Load all data in parallel (world, messages, codex, characters)
  const [world, messages, rawCodex, rawCharacters, storyState, persistedSummary, pinnedCanon] = await Promise.all([
    invoke<World>("world_get", { id: worldId }),
    invoke<Message[]>("session_list_messages", { session_id: sessionId }),
    invoke<CodexEntryRaw[]>("codex_list_for_world", { world_id: worldId }),
    invoke<CharacterRaw[]>("character_list", { world_id: worldId }),
    invoke<StoryState>("story_state_get", { session_id: sessionId }),
    invoke<string>("session_get_summary", { session_id: sessionId }).catch(() => ""),
    invoke<{ content: string }[]>("story_pinned_canon_list", { session_id: sessionId }).catch(() => []),
  ]);
  throwIfAborted();
  const promptMessages = replaceMessageId
    ? messages.filter((message) => message.id !== replaceMessageId)
    : messages;
  const recalledScenes = await invoke<{ message_id: string; content: string; score: number }[]>("story_memory_recall", {
    session_id: sessionId,
    query: userAction,
    limit: 4,
  }).catch((error) => {
    console.warn("Semantic memory recall unavailable; continuing without recalled scenes:", error instanceof Error ? error.message : String(error));
    return [];
  });

  // 2. Parse codex triggers (stored as JSON text in SQLite)
  const codexEntries = parseCodexEntries(rawCodex);

  // 3. Build recent text for codex matching
  //    Use the user action + last 2 messages to detect trigger keywords.
  const recentText = (
    userAction +
    " " +
      promptMessages
      .slice(-2)
      .map((m) => m.content)
      .join(" ")
  ).toLowerCase();

  // 4. Match codex entries against recent context
  const matched = matchCodex(codexEntries, recentText);

  // 4b. Build character context for the prompt.
  //     Each character is listed by name, role, backstory, and traits so the
  //     narrator can refer to them naturally during the story.
  let characterContext = "";
  if (rawCharacters && rawCharacters.length > 0) {
    const lines = rawCharacters.map((c) => {
      let traitsList: string[] = [];
      try {
        const parsed = JSON.parse(c.traits);
        traitsList = Array.isArray(parsed) ? parsed.filter((trait): trait is string => typeof trait === "string") : [];
      } catch { /* ignore */ }
      const parts = [`- ${sanitizeStrictForPrompt(capText(c.name, 120))} (${sanitizeStrictForPrompt(capText(c.role, 80))})`];
      if (c.backstory) parts.push(`: ${sanitizeStrictForPrompt(capText(c.backstory, 900))}`);
      if (traitsList.length > 0) parts.push(` [${traitsList.map((trait) => sanitizeStrictForPrompt(capText(trait, 120))).join(", ")}]`);
      return parts.join("");
    });
    characterContext = capText(`\nCharacters in this world:\n${lines.join("\n")}\n`, 7000);
  }

  // 4c. Rolling memory: truncate old messages when context is too long.
  //     Keep last 8 messages + a "story so far" summary built from older ones.
  const MAX_RECENT_MSGS = 8;
  let memorySummary = capText(summary || persistedSummary || "", 5000);
  let recentMessages = promptMessages;
  if (promptMessages.length > MAX_RECENT_MSGS) {
    const oldMessages = promptMessages.slice(0, promptMessages.length - MAX_RECENT_MSGS);
    const recent = promptMessages.slice(-MAX_RECENT_MSGS);
    const oldContent = oldMessages.map((m) => m.content).join(" ");
    // Keep both the setup and the latest consequences. Keeping only the
    // beginning made long stories forget their most recent turning point.
    const fallbackSummary = oldContent.length > 5000
      ? oldContent.slice(0, 2400) + " … " + oldContent.slice(-2400)
      : oldContent;
    memorySummary = capText(persistedSummary || fallbackSummary, 5000);
    recentMessages = recent;
  }

  // 5. Build system prompt via builder (respects preferences), inject
  //    character context, then assemble the full prompt.
  const contextSize = clampContextSize(preferences?.["contextSize"], useApp.getState().hardware?.ram_gb);
  const maxOutputTokens = clampOutputTokens(genParams?.max_tokens);
  const turnNumber = Math.floor((promptMessages.length + 1) / 2) + 1;
  const narrationBudget = getNarrationBudget(
    preferences?.["narrative_style"] ?? "default",
    userAction,
    turnNumber,
  );
  const systemPrompt = buildSystemFor(
    {
      name: capText(world.name, 240),
      genre: sanitizeForPrompt(capText(world.genre, 120)),
      description: capText(world.description ?? "", 6000),
      system_prompt: capText(world.system_prompt, 12_000),
    },
    preferences,
    narrationBudget,
  );
  const enrichedSystemPrompt = characterContext
    ? systemPrompt + "\n" + characterContext
    : systemPrompt;
  const boundedMatched = matched.map((entry) => ({
    ...entry,
    content: capText(entry.content, 1800),
  }));
  const storyStateJson = capText(JSON.stringify(storyState, null, 2), 6000);
  const pinnedCanonContext = capText(
    pinnedCanon.map((item) => `- ${item.content}`).join("\n"),
    6000,
  );
  const inputBudget = Math.max(
    512,
    contextSize - maxOutputTokens - Math.ceil(contextSize * CONTEXT_SAFETY_RATIO),
  );
  const semanticMemory = boundSemanticMemory(recalledScenes, Math.floor(inputBudget * 0.2));
  recentMessages = trimContext(recentMessages, {
    systemPrompt: enrichedSystemPrompt,
    summary: memorySummary,
    codexContext: `${storyStateJson}\n${pinnedCanonContext}\n${boundedMatched.map((entry) => entry.content).join("\n")}\n${semanticMemory ?? ""}`,
    userMessage: capText(userAction, 4000),
    contextTokenTarget: inputBudget,
    historyFloor: 0,
  });
  const prompt = buildPrompt({
    systemPrompt: enrichedSystemPrompt,
    summary: memorySummary,
    matchedCodex: boundedMatched,
    recentMessages: recentMessages,
    userAction: capText(userAction, 4000),
    turnNumber,
    openingInstruction: promptMessages.length === 0 ? OPENING_INSTRUCTION : undefined,
    storyState: storyStateJson,
    pinnedCanon: pinnedCanonContext || undefined,
    semanticMemory,
  });

  if (estimateTokens(prompt) > inputBudget) {
    console.warn(`Narration prompt exceeds input budget: ${estimateTokens(prompt)} > ${inputBudget}`);
  }

  // 6. Generate via local Ollama (streaming with event fallback)
  let rawOutput: string;
  const requestId = crypto.randomUUID();
  const invokeArgs: Record<string, unknown> = { prompt, world_id: worldId, request_id: requestId };
  if (genParams?.temperature !== undefined) invokeArgs.temperature = genParams.temperature;
  if (genParams?.top_p !== undefined) invokeArgs.top_p = genParams.top_p;
  const narrationTokenCap = Math.ceil(narrationBudget.hardCap * 2) + 64;
  invokeArgs.max_tokens = Math.min(maxOutputTokens, narrationTokenCap);
  // The first request may have completed before an IPC/stream error surfaced.
  // Never repeat it blindly and duplicate local compute or cloud billing.
  if (signal?.aborted) throw new Error("Narration cancelled");
  const cancelGeneration = () => invoke("narration_cancel", { request_id: requestId }).catch(() => {});
  signal?.addEventListener("abort", cancelGeneration, { once: true });
  try {
    rawOutput = await invoke<string>("narration_generate_stream", invokeArgs);
  } finally {
    signal?.removeEventListener("abort", cancelGeneration);
  }

  // 7. Post-process (strip markdown, clean verbal tics)
  const clean = limitNarrationWords(postProcess(rawOutput), narrationBudget.hardCap);
  if (!clean.trim()) {
    throw new Error("Narrator returned an empty response");
  }
  // Cancellation can arrive after inference completes but before persistence.
  // Do not write a turn that the caller has already abandoned.
  throwIfAborted();

  if (replaceMessageId) {
    // Keep the previous derived state intact until generation has succeeded.
    // A failed regeneration must not destroy continuity for the old scene.
    await invoke("narration_prepare_regeneration", {
      session_id: sessionId,
      message_id: replaceMessageId,
    });
  }

  // 8. Persist messages (user then narrator). Once the first persistence
  // mutation starts, finish the pair even if cancellation arrives; aborting
  // between these calls would leave a durable user turn without its narrator.
  let userId: string | undefined;
  if (persistUserAction) {
    userId = await invoke<string>("session_append_message", {
      session_id: sessionId,
      role: "user",
      content: userAction,
    });
  }
  let narratorId: string;
  if (replaceMessageId) {
    narratorId = await invoke<string>("message_replace_content", {
      message_id: replaceMessageId,
      session_id: sessionId,
      content: clean,
    });
  } else {
    narratorId = await invoke<string>("session_append_message", {
      session_id: sessionId,
      role: "narrator",
      content: clean,
    });
  }

  const anchorState: StoryState = {
    ...storyState,
    source: "model",
    turn: Math.max(storyState.turn ?? 0, turnNumber),
    last_scene: clean.slice(-2000),
  };

  const continuity = (async (): Promise<StoryState> => {
    let nextState = anchorState;
    if (shouldExtractStoryState(turnNumber, userAction, clean, preferences?.["performanceProfile"])) {
      try {
        nextState = await invoke<StoryState>("story_state_extract", {
          user_action: userAction,
          narration: clean,
          current_state: storyState,
          world_id: worldId,
        });
      } catch {
        // Extraction is an enhancement, never a reason to lose a story turn.
      }
    }

    await invoke("story_state_set", {
      session_id: sessionId,
      story_state: nextState,
    }).catch(() => {});

    // Index last so low-memory machines do not switch chat → embedding → chat
    // before Story State extraction. The next recall can reuse the embed runner.
    await invoke("story_memory_index", {
      session_id: sessionId,
      message_id: narratorId,
      content: capText(`Player action: ${userAction}\nNarrator: ${clean}`, 20_000),
    }).catch(() => {});

    if (promptMessages.length > MAX_RECENT_MSGS) {
      const durableSummary = capText(
        [memorySummary, `Latest consequence: ${clean.slice(-1200)}`]
          .filter(Boolean)
          .join("\n"),
        6000,
      );
      await invoke("session_update_summary", {
        session_id: sessionId,
        summary: durableSummary,
      }).catch(() => {});
    }
    return nextState;
  })();
  return {
    content: clean,
    memorySummary: promptMessages.length > MAX_RECENT_MSGS ? memorySummary : undefined,
    narratorId,
    userId,
    storyState: anchorState,
    continuity,
  };
}

/** Serialize every operation that can mutate one session, including the
 * background continuity pass. The lock is installed before generation starts
 * so two rapid submissions cannot both read the same prior history. */
export async function generateNarration(
  ...args: Parameters<typeof generateNarrationUnlocked>
): ReturnType<typeof generateNarrationUnlocked> {
  const sessionId = args[0];
  await waitForContinuity(sessionId);

  let release!: () => void;
  const lock = new Promise<void>((resolve) => { release = resolve; });
  continuityJobs.set(sessionId, lock);

  const cleanup = () => {
    release();
    if (continuityJobs.get(sessionId) === lock) continuityJobs.delete(sessionId);
  };

  try {
    const result = await generateNarrationUnlocked(...args);
    void result.continuity.then(cleanup, cleanup);
    return result;
  } catch (error) {
    cleanup();
    throw error;
  }
}
