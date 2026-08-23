/**
 * NarrationScreen — continuous vertical scroll narrative reader.
 *
 * Renders story messages in a flowing text layout (like a web novel /
 * manhwa scroll): narrator paragraphs as normal text, user actions as
 * indented/muted blocks. No chat bubbles, no avatars, no companion UI.
 *
 * Responsibilities:
 *   - On mount, load existing messages via `session_list_messages`.
 *   - Accept user input at the bottom to continue the story.
 *   - Call `generateNarration` to run the full prompt pipeline + persist.
 *   - Auto-scroll to the latest content.
 *   - Surface local model and generation errors from the backend.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ArrowLeft, Check, ChevronDown, Download, Edit3, FileDown, ImageIcon, Loader2, Printer, Send, MessageSquare, PenLine, Quote, X } from "lucide-react";
import { Slider } from "@/components/ui/slider";
import { toast } from "sonner";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useApp } from "@/lib/store";
import { imageGenerate, messageUpdate, modelPrewarmActive, modelUnloadActive, storyStateGet, systemEnergyGet, systemStatus, type StoryState, type SystemStatus, type VisualBible } from "@/lib/tauri";
import { generateNarration } from "@/lib/narration/generate";
import { TutorialOverlay } from "@/components/narration/TutorialOverlay";
import { OrnamentDivider } from "@/components/ui/ornament";
import { buildSceneImagePrompt, type SceneImageStyle } from "@/lib/narration/imagePrompt";
import { useTranslation } from "react-i18next";

// ── Types ──────────────────────────────────────────────────────────

/** A single turn in the story session. Mirrors the Rust Message struct
 *  plus optimistic messages generated locally before the backend round-trip. */
interface NarrationMessage {
  id: string;
  role: "narrator" | "user";
  content: string;
  created_at: string;
}

const ADVANCED_PRESET_KEYS = [
  "generation_temperature", "generation_top_p", "generation_max_tokens", "contextSize",
  "semanticMemoryEnabled", "performanceProfile", "story_image_frequency", "story_image_quality",
  "story_image_style", "story_image_max_auto", "autoImagesAcOnly",
] as const;

type AdvancedPreset = { name: string; values: Partial<Record<(typeof ADVANCED_PRESET_KEYS)[number], string>> };

function readAdvancedPresets(raw: string | undefined): AdvancedPreset[] {
  try {
    const parsed = JSON.parse(raw ?? "[]");
    return Array.isArray(parsed) ? parsed.filter((item): item is AdvancedPreset => typeof item?.name === "string" && item.values && typeof item.values === "object").slice(0, 12) : [];
  } catch { return []; }
}

// ── Helpers ────────────────────────────────────────────────────────

let msgCounter = 0;
function tempId(): string {
  return `msg-${Date.now()}-${++msgCounter}`;
}

async function loadScenes(sessionId: string, messages: NarrationMessage[]): Promise<Record<number, string>> {
  try {
    const saved = await invoke<{ message_id: string; path: string }[]>("scene_image_list", { session_id: sessionId });
    return Object.fromEntries(saved.flatMap(({ message_id, path }) => {
      const index = messages.findIndex((message) => message.id === message_id);
      return index >= 0 ? [[index, convertFileSrc(path)]] : [];
    }));
  } catch {
    return {};
  }
}

async function saveScene(sessionId: string, messageId: string, dataUri: string, metadata?: object): Promise<string | null> {
  try {
    const path = await invoke<string>("scene_image_save", {
      session_id: sessionId,
      message_id: messageId,
      image_b64: dataUri,
      metadata: metadata ? JSON.stringify(metadata) : undefined,
    });
    return convertFileSrc(path);
  } catch {
    return null;
  }
}

async function imageSourceForExport(source: string): Promise<string | null> {
  if (/^data:image\/(?:png|jpeg|webp);base64,/i.test(source)) return source;
  try {
    const response = await fetch(source);
    if (!response.ok) return null;
    const blob = await response.blob();
    return await new Promise((resolve) => {
      const reader = new FileReader();
      reader.onload = () => resolve(typeof reader.result === "string" ? reader.result : null);
      reader.onerror = () => resolve(null);
      reader.readAsDataURL(blob);
    });
  } catch {
    return null;
  }
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (character) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  })[character] ?? character);
}

// ── Component ──────────────────────────────────────────────────────

export function NarrationScreen() {
  const { t } = useTranslation();
  // Store access
  const currentSessionId = useApp((s) => s.current_session_id);
  const currentWorldId = useApp((s) => s.current_world_id);
  const currentWorld = useApp((s) => s.worlds.find((w) => w.id === s.current_world_id));
  const setActiveView = useApp((s) => s.setActiveView);
  const preferences = useApp((s) => s.preferences);
  const textSize = preferences["textSize"] || "normal";
  const fontSize = textSize === "small" ? "13px" : textSize === "large" ? "16px" : "14px";
  const narrationFont = preferences["narrationFont"] || "serif";

  // Local state
  const [messages, setMessages] = useState<NarrationMessage[]>([]);
  const [input, setInput] = useState("");
  const [mode, setMode] = useState<"do" | "say" | "story" | "visualize">("do");
  const [loading, setLoading] = useState(true);
  const [reloadKey, setReloadKey] = useState(0);
  const generateInitialStoryRef = useRef<() => void>(() => {});
  const [generating, setGenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [streamingText, setStreamingText] = useState<string | null>(null);
  const streamBufferRef = useRef("");
  const streamFrameRef = useRef<number | null>(null);
  const streamTimerRef = useRef<number | null>(null);
  const [sceneImages, setSceneImages] = useState<Record<number, string>>({});
  const [sceneFailures, setSceneFailures] = useState<Record<number, number>>({});
  const [sceneErrors, setSceneErrors] = useState<Record<number, string>>({});
  const [sceneSeeds, setSceneSeeds] = useState<Record<number, number>>({});
  const [visualizedImage, setVisualizedImage] = useState<string | null>(null);
  const [generatingScene, setGeneratingScene] = useState<number | null>(null);
  const [sceneProgress, setSceneProgress] = useState(0);
  const [sceneElapsed, setSceneElapsed] = useState(0);
  const [sceneStyle, setSceneStyle] = useState<SceneImageStyle>("cinematic");
  const SCENE_STYLES: SceneImageStyle[] = ["anime", "realistic", "watercolor", "ink", "cinematic", "dark-fantasy", "manga"];
  const [memorySummary, setMemorySummary] = useState<string | null>(null);
  const [generationPanelOpen, setGenerationPanelOpen] = useState(false);
  const [presetName, setPresetName] = useState("");
  const lastUserActionRef = useRef<string>("");
  const initialGenerationSessionRef = useRef<string | null>(null);
  const [storyState, setStoryState] = useState<StoryState | null>(null);
  const [visualBible, setVisualBible] = useState<VisualBible | null>(null);
  const [narratorStatus, setNarratorStatus] = useState<SystemStatus | null>(null);
  const autoImageCheckRef = useRef<number | null>(null);
  const narrationAbortRef = useRef<AbortController | null>(null);
  const sceneGenerationRef = useRef(0);
  const [editingMessageId, setEditingMessageId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");
  const workspaceJumpToMessage = useApp((s) => s.workspace_jump_to_message);
  const setWorkspaceMessages = useApp((s) => s.setWorkspaceMessages);
  const setWorkspaceSceneImages = useApp((s) => s.setWorkspaceSceneImages);
  const setWorkspaceStoryState = useApp((s) => s.setWorkspaceStoryState);
  const clearWorkspaceJump = useApp((s) => s.clearWorkspaceJump);

  useEffect(() => {
    if (!currentWorldId) {
      setVisualBible(null);
      return;
    }
    let cancelled = false;
    setVisualBible(null);
    invoke<VisualBible>("world_visual_bible_get", { world_id: currentWorldId })
      .then((bible) => { if (!cancelled) setVisualBible(bible); })
      .catch(() => { if (!cancelled) setVisualBible(null); });
    return () => { cancelled = true; };
  }, [currentWorldId]);

  useEffect(() => {
    let cancelled = false;
    const refreshStatus = () => systemStatus()
      .then((status) => { if (!cancelled) setNarratorStatus(status); })
      .catch(() => { if (!cancelled) setNarratorStatus(null); });
    refreshStatus();
    const interval = window.setInterval(refreshStatus, 15_000);
    return () => { cancelled = true; window.clearInterval(interval); };
  }, [currentSessionId]);

  // Generation params from preferences
  const genTemperature = (() => {
    const v = parseFloat(preferences["generation_temperature"] ?? "");
    return isNaN(v) ? 0.8 : v;
  })();
  const genTopP = (() => {
    const v = parseFloat(preferences["generation_top_p"] ?? "");
    return isNaN(v) ? 0.9 : v;
  })();
  const genMaxTokens = (() => {
    const v = parseInt(preferences["generation_max_tokens"] ?? "", 10);
    return isNaN(v) ? 2048 : v;
  })();
  const updatePreference = useApp((s) => s.updatePreference);
  const imageFrequency = preferences["story_image_frequency"] || (preferences["auto_scene_images"] === "true" ? "every" : "off");
  const imageQuality = preferences["story_image_quality"] || "balanced";
  const imageMaxAuto = parseInt(preferences["story_image_max_auto"] || "10", 10) || 0;
  const autoSceneImages = imageFrequency !== "off";
  const performanceProfile = preferences["performanceProfile"] || "balanced";
  const advancedControls = preferences["generation_advanced"] === "true";
  const advancedPresets = readAdvancedPresets(preferences["generation_advanced_presets"]);

  const saveAdvancedPreset = useCallback(() => {
    const name = presetName.trim();
    if (!name) return;
    const values = Object.fromEntries(ADVANCED_PRESET_KEYS.map((key) => [key, preferences[key] ?? ""]));
    const next = [...advancedPresets.filter((preset) => preset.name.toLowerCase() !== name.toLowerCase()), { name, values }].slice(-12);
    updatePreference("generation_advanced_presets", JSON.stringify(next));
    setPresetName("");
  }, [advancedPresets, preferences, presetName, updatePreference]);

  const loadAdvancedPreset = useCallback((preset: AdvancedPreset) => {
    Object.entries(preset.values).forEach(([key, value]) => updatePreference(key, value));
  }, [updatePreference]);

  const applyPerformanceProfile = useCallback((profile: "efficient" | "balanced" | "quality") => {
    const values = {
      efficient: {
        contextSize: "4096", generation_max_tokens: "512", semanticMemoryEnabled: "false",
        lowPowerMode: "true", story_image_frequency: "important", autoImagesAcOnly: "true",
      },
      balanced: {
        contextSize: "8192", generation_max_tokens: "768", semanticMemoryEnabled: "false",
        lowPowerMode: "false", story_image_frequency: "every3", autoImagesAcOnly: "true",
      },
      quality: {
        contextSize: "16384", generation_max_tokens: "1024", semanticMemoryEnabled: "true",
        lowPowerMode: "false", story_image_frequency: "every2", autoImagesAcOnly: "true",
      },
    }[profile];
    updatePreference("performanceProfile", profile);
    Object.entries(values).forEach(([key, value]) => updatePreference(key, value));
  }, [updatePreference]);

  const flushStreamBuffer = useCallback((isCurrent: () => boolean) => {
    streamFrameRef.current = null;
    streamTimerRef.current = null;
    if (isCurrent()) setStreamingText(streamBufferRef.current || null);
  }, []);

  const beginTokenStream = useCallback(async (isCurrent: () => boolean) => {
    streamBufferRef.current = "";
    setStreamingText(null);
    return listen<string>("narration:token", (event) => {
      if (!isCurrent()) return;
      streamBufferRef.current += event.payload;
      if (streamFrameRef.current === null && streamTimerRef.current === null) {
        // A 50 ms window batches fast token bursts while remaining visually
        // smoother than the model's typical token cadence.
        streamTimerRef.current = window.setTimeout(() => {
          streamFrameRef.current = requestAnimationFrame(() => flushStreamBuffer(isCurrent));
        }, 50);
      }
    }).catch(() => undefined);
  }, [flushStreamBuffer]);

  const clearTokenStream = useCallback(() => {
    if (streamFrameRef.current !== null) cancelAnimationFrame(streamFrameRef.current);
    if (streamTimerRef.current !== null) window.clearTimeout(streamTimerRef.current);
    streamFrameRef.current = null;
    streamTimerRef.current = null;
    streamBufferRef.current = "";
    setStreamingText(null);
  }, []);

  const stopNarration = useCallback(() => {
    const controller = narrationAbortRef.current;
    controller?.abort();
    // Abort makes isCurrentNarration false, so the async finally block cannot
    // be the only place that clears the UI lock. Keep the controller owned by
    // the request until finally runs, allowing persistence cleanup to finish.
    if (controller) {
      clearTokenStream();
      setGenerating(false);
    }
  }, [clearTokenStream]);

  const isCurrentNarration = useCallback((sessionId: string, controller: AbortController) => (
    narrationAbortRef.current === controller
    && !controller.signal.aborted
    && useApp.getState().current_session_id === sessionId
  ), []);

  useEffect(() => {
    narrationAbortRef.current?.abort();
    clearTokenStream();
    setGenerating(false);
    lastUserActionRef.current = "";
  }, [currentSessionId, clearTokenStream]);

  const prewarmStartedRef = useRef(false);
  const prewarmNarrator = useCallback(() => {
    if (prewarmStartedRef.current || !currentSessionId) return;
    prewarmStartedRef.current = true;
    modelPrewarmActive().catch(() => {
      prewarmStartedRef.current = false;
    });
  }, [currentSessionId]);

  useEffect(() => {
    prewarmStartedRef.current = false;
  }, [currentSessionId]);

  useEffect(() => () => {
    modelUnloadActive().catch(() => {});
  }, []);

  // Abort any in-flight narration when leaving the screen entirely (the
  // session-switch effect above only covers same-screen session changes).
  useEffect(() => () => {
    narrationAbortRef.current?.abort();
  }, []);

  useEffect(() => {
    const configured = preferences["story_image_style"] as SceneImageStyle | undefined;
    if (configured && SCENE_STYLES.includes(configured)) setSceneStyle(configured);
  }, [preferences["story_image_style"]]);

  // Refs
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [isNearBottom, setIsNearBottom] = useState(true);

  useEffect(() => {
    if (generatingScene === null) {
      setSceneElapsed(0);
      return;
    }
    const started = Date.now();
    const timer = window.setInterval(() => setSceneElapsed(Date.now() - started), 1000);
    return () => window.clearInterval(timer);
  }, [generatingScene]);

  useEffect(() => setWorkspaceMessages(messages), [messages, setWorkspaceMessages]);
  useEffect(() => setWorkspaceSceneImages(sceneImages), [sceneImages, setWorkspaceSceneImages]);
  useEffect(() => setWorkspaceStoryState(storyState), [storyState, setWorkspaceStoryState]);
  // ── Auto-scroll ──────────────────────────────────────────────────
  const scrollToBottom = useCallback((smooth = true) => {
    requestAnimationFrame(() => {
      if (scrollRef.current) {
        scrollRef.current.scrollTo({
          top: scrollRef.current.scrollHeight,
          behavior: smooth ? "smooth" : "instant",
        });
      }
    });
  }, []);

  // Jump to a specific message by index
  const jumpToMessage = useCallback((msgIndex: number) => {
    if (!scrollRef.current) return;
    // Find the DOM element for this message
    const el = scrollRef.current.querySelector(`[data-msg-idx="${msgIndex}"]`);
    if (el) {
      el.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }, []);

  useEffect(() => {
    if (workspaceJumpToMessage !== null) {
      jumpToMessage(workspaceJumpToMessage);
      clearWorkspaceJump();
    }
  }, [workspaceJumpToMessage, clearWorkspaceJump, jumpToMessage]);

  // Track scroll position to avoid fighting user
  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const threshold = 100;
    const near = el.scrollHeight - el.scrollTop - el.clientHeight < threshold;
    setIsNearBottom(near);
  }, []);

  // ── Load persisted messages on mount ─────────────────────────────
  useEffect(() => {
    if (!currentSessionId) {
      setLoading(false);
      setError("No active session. Select a world from the library first.");
      return;
    }

    let cancelled = false;
    const sessionId = currentSessionId;
    const worldId = currentWorldId;
    const isCurrentLoad = () => !cancelled
      && useApp.getState().current_session_id === sessionId
      && useApp.getState().current_world_id === worldId;
    setLoading(true);
    setError(null);
    setMemorySummary(null);
    setStoryState(null);
    setSceneImages({});
    setSceneFailures({});
    setSceneErrors({});
    setSceneSeeds({});
    setVisualizedImage(null);
    setGeneratingScene(null);
    setEditingMessageId(null);
    setEditingText("");
    sceneGenerationRef.current += 1;
    autoImageCheckRef.current = null;

    invoke<NarrationMessage[]>("session_list_messages", {
      session_id: currentSessionId,
    })
      .then((msgs) => {
        if (!isCurrentLoad()) return;
        setMessages(msgs);
        storyStateGet(sessionId)
          .then((state) => { if (isCurrentLoad()) setStoryState(state); })
          .catch(() => { if (isCurrentLoad()) setStoryState(null); });
        // Restore persisted scene images
        void loadScenes(sessionId, msgs).then((images) => {
          if (isCurrentLoad()) setSceneImages(images);
        });
        setLoading(false);
        // A new session should open with a generated scene, not an empty
        // input prompt waiting for the player to send a first message.
        if (msgs.length === 0 && initialGenerationSessionRef.current !== currentSessionId) {
          initialGenerationSessionRef.current = currentSessionId;
          void generateInitialStoryRef.current();
        }
        scrollToBottom();
      })
      .catch((e) => {
        if (cancelled) return;
        setError((e as { message?: string })?.message ?? String(e));
        setMessages([]);
        setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [currentSessionId, scrollToBottom, reloadKey]);

  const generateInitialStory = useCallback(async () => {
    if (!currentSessionId || !currentWorldId || generating) return;
    setError(null);
    setGenerating(true);
    const controller = new AbortController();
    narrationAbortRef.current = controller;
    const isCurrent = () => isCurrentNarration(currentSessionId, controller);
    const unlisten = await beginTokenStream(isCurrent);
    try {
      const { content, memorySummary: memSummary, narratorId, storyState: nextState, continuity } = await generateNarration(
        currentSessionId,
        currentWorldId,
        "Begin the story with a compelling opening scene. Do not ask the player what they want to do.",
        undefined,
        { temperature: genTemperature, top_p: genTopP, max_tokens: genMaxTokens },
        preferences,
        false,
        undefined,
        controller.signal,
      );
      if (!isCurrent()) return;
      if (memSummary) setMemorySummary(memSummary);
      setStoryState(nextState);
      void continuity.then((resolved) => {
        if (isCurrent()) setStoryState(resolved);
      });
      setMessages([{
        id: narratorId,
        role: "narrator",
        content,
        created_at: new Date().toISOString(),
      }]);
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : String(e);
      if (isCurrent() && !message.toLowerCase().includes("cancel")) setError(message);
    } finally {
      unlisten?.();
      if (isCurrent()) {
        clearTokenStream();
        setGenerating(false);
        narrationAbortRef.current = null;
      }
    }
  }, [currentSessionId, currentWorldId, generating, genTemperature, genTopP, genMaxTokens, preferences, beginTokenStream, clearTokenStream, isCurrentNarration]);
  // Keep the ref in sync so the message-load effect can call the latest
  // callback without taking it as a dependency (its identity changes with
  // `generating`, which would churn reloads).
  generateInitialStoryRef.current = generateInitialStory;

  // ── Auto-scroll when messages are added ──────────────────────────
  useEffect(() => {
    // Only auto-scroll if user is near bottom (not reading history)
    if (!loading && isNearBottom) scrollToBottom();
  }, [messages.length, loading, isNearBottom, scrollToBottom]);

  // ── Submit user action ───────────────────────────────────────────
  const handleVisualize = useCallback(async () => {
    const text = input.trim();
    if (!text || !currentWorldId || generating || generatingScene !== null) return;
    setGeneratingScene(-1);
    setSceneProgress(0);
    const unlisten = await listen<number>("image:render_progress", (event) => setSceneProgress(event.payload)).catch(() => undefined);
    try {
      const prompt = buildSceneImagePrompt(
        text,
        currentWorld?.name || "Untitled story",
        currentWorld?.genre || "",
        storyState,
        sceneStyle,
        visualBible,
      );
      const result = await imageGenerate({
        prompt,
        style: sceneStyle,
        quality: imageQuality,
        world_id: currentWorldId,
      });
      if (!result?.image_b64 || result.image_b64.length < 100) throw new Error("Image generation returned no data");
      const dataUri = `data:image/png;base64,${result.image_b64}`;
      setVisualizedImage(dataUri);
      setInput("");
    } catch (error) {
      console.error("visualization failed:", error);
      toast.error(t("ui.visualization-failed", "Visualization failed. Check that the image model is installed."));
    } finally {
      unlisten?.();
      setGeneratingScene(null);
      setSceneProgress(0);
    }
  }, [input, currentWorldId, generating, generatingScene, currentWorld, storyState, sceneStyle, visualBible, imageQuality]);

  const handleSubmit = useCallback(async (actionOverride?: string) => {
    if (mode === "visualize" && !actionOverride) {
      await handleVisualize();
      return;
    }
    const text = (actionOverride ?? input).trim();
    if (!text || !currentSessionId || !currentWorldId || generating) return;

    // Format text based on mode
    const formattedText = mode === "say"
      ? `"${text}"`
      : mode === "story"
        ? `[NARRATE: ${text}]`
        : text;

    setError(null);
    setVisualizedImage(null);

    // Save for regenerate
    lastUserActionRef.current = formattedText;

    // Optimistically add the user's action so it appears instantly.
    const userMsg: NarrationMessage = {
      id: tempId(),
      role: "user",
      content: formattedText,
      created_at: new Date().toISOString(),
    };
    setMessages((prev) => [...prev, userMsg]);
    setGenerating(true);
    const controller = new AbortController();
    narrationAbortRef.current = controller;
    const isCurrent = () => isCurrentNarration(currentSessionId, controller);

    // Clear input only after optimistic add succeeds
    setInput("");

    // Set up streaming event listener
    const unlisten = await beginTokenStream(isCurrent);

    try {
      // generateNarration handles the full pipeline
      const { content: narratorContent, memorySummary: memSummary, narratorId, userId, storyState: nextState, continuity } = await generateNarration(
        currentSessionId,
        currentWorldId,
        formattedText,
        undefined,
        { temperature: genTemperature, top_p: genTopP, max_tokens: genMaxTokens },
        preferences,
        true,
        undefined,
        controller.signal,
      );

      if (!isCurrent()) return;
      if (memSummary) setMemorySummary(memSummary);
      setStoryState(nextState);
      void continuity.then((resolved) => {
        if (isCurrent()) setStoryState(resolved);
      });

      // Add the narrator's response to local state.
      const narratorMsg: NarrationMessage = {
        id: tempId(),
        role: "narrator",
        content: narratorContent,
        created_at: new Date().toISOString(),
      };
      setMessages((prev) => [
        ...prev.map((message) => message.id === userMsg.id && userId
          ? { ...message, id: userId }
          : message),
        { ...narratorMsg, id: narratorId },
      ]);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      if (!isCurrent()) return;
      if (!msg.toLowerCase().includes("cancel")) setError(msg);
      setMessages((prev) => prev.filter((message) => message.id !== userMsg.id));
      // Restore last action on failure so user doesn't lose typed text
      if (lastUserActionRef.current) setInput(lastUserActionRef.current);
    } finally {
      unlisten?.();
      if (isCurrent()) {
        clearTokenStream();
        setGenerating(false);
        narrationAbortRef.current = null;
        // Focus back on the input for the next action.
        inputRef.current?.focus();
      }
    }
  }, [input, mode, handleVisualize, currentSessionId, currentWorldId, generating, genTemperature, genTopP, genMaxTokens, preferences, beginTokenStream, clearTokenStream, isCurrentNarration]);

  // ── Regenerate last narrator message ──────────────────────────────
  const handleRegenerate = useCallback(async (variant = "") => {
    const text = lastUserActionRef.current || [...messages].reverse().find((message) => message.role === "user")?.content || "";
    const previousNarratorId = messages[messages.length - 1]?.role === "narrator"
      ? messages[messages.length - 1].id
      : undefined;
    const previousNarrator = messages[messages.length - 1]?.role === "narrator"
      ? messages[messages.length - 1]
      : undefined;
    if (!text || !currentSessionId || !currentWorldId || generating || !previousNarratorId) return;

    // Remove last narrator message from local state
    setMessages((prev) => {
      const idx = prev.length - 1;
      if (idx >= 0 && prev[idx].role === "narrator") {
        return prev.slice(0, idx);
      }
      return prev;
    });
    setGenerating(true);
    const controller = new AbortController();
    narrationAbortRef.current = controller;
    const isCurrent = () => isCurrentNarration(currentSessionId, controller);

    // Set up streaming listener (same pattern as handleSubmit)
    const unlisten = await beginTokenStream(isCurrent);

    try {
      const generationAction = variant
        ? `${text}\n\n[Rewrite direction: ${variant}]`
        : text;
      const { content: narratorContent, memorySummary: memSummary, narratorId, storyState: nextState, continuity } = await generateNarration(
        currentSessionId,
        currentWorldId,
        generationAction,
        undefined,
        { temperature: genTemperature, top_p: genTopP, max_tokens: genMaxTokens },
        preferences,
        false,
        previousNarratorId,
        controller.signal,
      );
      if (!isCurrent()) return;
      if (memSummary) setMemorySummary(memSummary);
      setStoryState(nextState);
      void continuity.then((resolved) => {
        if (isCurrent()) setStoryState(resolved);
      });
      const narratorMsg: NarrationMessage = {
        id: narratorId,
        role: "narrator",
        content: narratorContent,
        created_at: new Date().toISOString(),
      };
      setMessages((prev) => [...prev, narratorMsg]);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      if (!isCurrent()) return;
      if (!msg.toLowerCase().includes("cancel")) setError(msg);
      if (previousNarrator) {
        setMessages((prev) => prev.some((message) => message.id === previousNarrator.id) ? prev : [...prev, previousNarrator]);
      }
      // Restore last action on failure so user doesn't lose typed text
      if (lastUserActionRef.current) setInput(lastUserActionRef.current);
    } finally {
      unlisten?.();
      if (isCurrent()) {
        clearTokenStream();
        setGenerating(false);
        narrationAbortRef.current = null;
        inputRef.current?.focus();
      }
    }
  }, [messages, currentSessionId, currentWorldId, generating, genTemperature, genTopP, genMaxTokens, preferences, beginTokenStream, clearTokenStream, isCurrentNarration]);

  const handleContinue = useCallback(() => {
    void handleSubmit("Continue the scene naturally from the current beat. Do not resolve the player's next decision.");
  }, [handleSubmit]);

  const beginMessageEdit = useCallback((message: NarrationMessage) => {
    setEditingMessageId(message.id);
    setEditingText(message.content);
  }, []);

  const cancelMessageEdit = useCallback(() => {
    setEditingMessageId(null);
    setEditingText("");
  }, []);

  const saveMessageEdit = useCallback(async () => {
    if (!editingMessageId || !editingText.trim()) return;
    const edited = messages.find((message) => message.id === editingMessageId);
    try {
      await messageUpdate(editingMessageId, editingText.trim());
      setMessages((prev) => prev.map((message) =>
        message.id === editingMessageId
          ? { ...message, content: editingText.trim() }
          : message,
      ));
      if (edited?.role === "user") lastUserActionRef.current = editingText.trim();
      cancelMessageEdit();
    } catch (e) {
      toast.error(t("ui.could-not-save-edited-message", "Could not save the edited message"), { description: String(e) });
    }
  }, [editingMessageId, editingText, messages, cancelMessageEdit]);

  // ── Key handler — Enter to submit, Shift+Enter for newline ───────
  const sendOnEnter = preferences["sendOnEnter"] !== "false";
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      // When sendOnEnter is off, Enter = newline, Cmd+Enter = submit
      if (e.key === "Enter" && !e.shiftKey) {
        if (!sendOnEnter && !e.metaKey && !e.ctrlKey) return; // allow native newline
        e.preventDefault();
        handleSubmit();
      }
    },
    [handleSubmit, sendOnEnter],
  );

  // ── Export story as markdown ─────────────────────────────────────
  const exportClipboard = useCallback(() => {
    if (messages.length === 0) return;
    const header = currentWorld
      ? `# ${currentWorld.name}\n\n`
      : "# Story Export\n\n";
    const body = messages
      .map((m, index) =>
        (m.role === "narrator" ? m.content : `> *${m.content}*`) +
        (sceneImages[index] ? `\n\n[Scene image ${index + 1}]` : "")
      )
      .join("\n\n");
    const full = header + body;
    navigator.clipboard.writeText(full).then(() => {
      toast(t("ui.story-copied", "Story copied"), {
        description: t("ui.messages-copied-to-clipboard", "{{count}} messages copied to clipboard.", { count: messages.length }),
        duration: 3000,
      });
    }).catch(() => {
      toast.error(t("ui.failed-to-copy-story", "Failed to copy story."));
    });
  }, [messages, currentWorld, sceneImages]);

  // ── Export story as printable HTML/PDF ───────────────────────────
  const exportPrint = useCallback(async () => {
    if (messages.length === 0) return;
    const title = currentWorld?.name || "Story Export";
    const bodyHTML = (await Promise.all(messages.map(async (m, index) => {
      const paragraph = m.role === "narrator"
        ? `<p class="narrator">${escapeHtml(m.content)}</p>`
        : `<p class="user">${escapeHtml(m.content)}</p>`;
      const image = sceneImages[index] ? await imageSourceForExport(sceneImages[index]) : null;
      return paragraph + (image ? `<figure><img src="${escapeHtml(image)}" alt="Scene ${index + 1}" style="max-width:100%;border-radius:8px;"><figcaption>Scene ${index + 1}</figcaption></figure>` : "");
    }))).join("\n");
    const html = `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>${escapeHtml(title)}</title>
<style>
  @page { margin: 2.5cm 2cm; }
  body { font-family: Georgia, "Times New Roman", serif; font-size: 12pt; line-height: 1.7; color: #1a1a1a; max-width: 650px; margin: 0 auto; padding: 2em; }
  h1 { font-family: Georgia, serif; font-size: 22pt; margin-bottom: 0.5em; }
  .narrator { margin: 0 0 0.8em 0; }
  .user { font-style: italic; color: #555; margin: 0 0 0.8em 0; padding-left: 1em; border-left: 2px solid #ccc; }
</style>
</head>
<body>
<h1>${escapeHtml(title)}</h1>
${bodyHTML}
</body>
</html>`;
    const win = window.open("", "_blank");
    if (win) {
      win.document.write(html);
      win.document.close();
      win.focus();
      win.print();
    } else {
      // Popup blocked — fall back to clipboard
      navigator.clipboard.writeText(html).then(() => {
        toast(t("ui.html-copied", "HTML copied"), { description: t("ui.popup-blocked-html-copied", "Popup was blocked. HTML copied to clipboard."), duration: 4000 });
      }).catch(() => toast.error(t("ui.failed-to-export", "Failed to export.")));
    }
  }, [messages, currentWorld, sceneImages]);

  // ── Generate scene image ──────────────────────────────────────────
  const handleGenerateScene = useCallback(async (msgIdx: number, automatic = false, seed?: number) => {
    if (generatingScene !== null) return;
    const sessionId = currentSessionId;
    const worldId = currentWorldId;
    const message = messages[msgIdx];
    if (!worldId || !message) return;
    const generationToken = ++sceneGenerationRef.current;
    const isCurrentScene = () => sceneGenerationRef.current === generationToken
      && useApp.getState().current_session_id === sessionId
      && useApp.getState().current_world_id === worldId;
    setGeneratingScene(msgIdx);
    setSceneProgress(0);
    const unlisten = await listen<number>("image:render_progress", (event) => setSceneProgress(event.payload)).catch(() => undefined);
    try {
      // Use last ~600 chars of narration for the prompt
       const promptText = buildSceneImagePrompt(
         message.content || "scene",
        currentWorld?.name || "Untitled story",
        currentWorld?.genre || "",
        storyState,
        sceneStyle,
        visualBible,
      );
      const result = await imageGenerate({
        prompt: promptText,
        style: sceneStyle,
         quality: imageQuality,
         seed,
         world_id: worldId,
       });
      if (!result?.image_b64 || result.image_b64.length < 100) {
        throw new Error("Image generation returned no data");
      }
      const dataUri = `data:image/png;base64,${result.image_b64}`;
      if (!isCurrentScene()) return;
       setSceneSeeds((previous) => ({ ...previous, [msgIdx]: result.seed }));
       if (!isCurrentScene()) return;
       if (sessionId) {
         const savedPath = await saveScene(sessionId, message.id, dataUri, result);
         if (!isCurrentScene()) return;
         setSceneImages((prev) => ({ ...prev, [msgIdx]: savedPath ?? dataUri }));
      } else {
        setSceneImages((prev) => ({ ...prev, [msgIdx]: dataUri }));
      }
      setSceneFailures((previous) => {
        const next = { ...previous };
        delete next[msgIdx];
        return next;
      });
      setSceneErrors((previous) => {
        const next = { ...previous };
        delete next[msgIdx];
        return next;
      });
    } catch (e) {
      console.error("scene generation failed:", e);
      setSceneFailures((previous) => ({ ...previous, [msgIdx]: (previous[msgIdx] ?? 0) + 1 }));
      setSceneErrors((previous) => ({ ...previous, [msgIdx]: e instanceof Error ? e.message : String(e) }));
      if (!automatic) toast.error(t("ui.scene-generation-failed", "Scene generation failed. Check that the image model is installed."));
    } finally {
      unlisten?.();
      setGeneratingScene(null);
      setSceneProgress(0);
    }
  }, [messages, generatingScene, sceneStyle, visualBible, imageQuality, currentSessionId, currentWorldId, currentWorld, storyState]);

  useEffect(() => {
    if (!autoSceneImages || generating || loading || generatingScene !== null) return;
    if (imageMaxAuto > 0 && Object.keys(sceneImages).length >= imageMaxAuto) return;
    const lastIndex = messages.length - 1;
    if (lastIndex < 0 || messages[lastIndex].role !== "narrator" || sceneImages[lastIndex]) return;
    const narratorCount = messages.slice(0, lastIndex + 1).filter((message) => message.role === "narrator").length;
    const important = /\b(attack|fight|battle|arrive|appears|reveal|discover|door|gate|blood|death|escape|kiss)\b/i.test(messages[lastIndex].content);
    const shouldGenerate = imageFrequency === "every"
      || (imageFrequency === "first" && narratorCount === 1)
      || (imageFrequency === "every2" && narratorCount % 2 === 0)
      || (imageFrequency === "every3" && narratorCount % 3 === 0)
      || (imageFrequency === "important" && important);
    if (!shouldGenerate) return;
    if ((sceneFailures[lastIndex] ?? 0) > 0) return;
    if (preferences["lowPowerMode"] === "true" || autoImageCheckRef.current === lastIndex) return;
    autoImageCheckRef.current = lastIndex;
    void systemEnergyGet()
      .then((energy) => {
        if (energy.thermally_constrained || (energy.battery_percent !== null && energy.battery_percent <= 20)) return;
        const acOnly = preferences["autoImagesAcOnly"] !== "false";
        if (acOnly && !energy.on_ac_power) return;
        return handleGenerateScene(lastIndex, true);
      })
      .finally(() => {
        if (autoImageCheckRef.current === lastIndex) autoImageCheckRef.current = null;
      });
  }, [autoSceneImages, imageFrequency, imageMaxAuto, generating, loading, generatingScene, messages, sceneImages, sceneFailures, handleGenerateScene, preferences["lowPowerMode"], preferences["autoImagesAcOnly"]]);

  // ── Save story as HTML file ──────────────────────────────────────
  const exportSaveFile = useCallback(async () => {
    if (messages.length === 0) return;
    const title = currentWorld?.name || "FPV Story";
    const bodyHTML = (await Promise.all(messages.map(async (m, index) => {
      const paragraph = m.role === "narrator"
        ? `<p class="narrator">${escapeHtml(m.content)}</p>`
        : `<p class="user">${escapeHtml(m.content)}</p>`;
      const dataUri = sceneImages[index] ? await imageSourceForExport(sceneImages[index]) : null;
      const image = dataUri
        ? `<figure><img src="${escapeHtml(dataUri)}" alt="Scene ${index + 1}" style="max-width:100%;margin:1em 0;border-radius:8px;"><figcaption style="font-size:0.85em;color:#888;">Scene ${index + 1}</figcaption></figure>`
        : "";
      return paragraph + image;
    }))).join("\n");
    const html = `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(title)}</title>
<style>
  body { font-family: Georgia, "Times New Roman", serif; font-size: 13pt; line-height: 1.75; color: #1a1a1a; max-width: 680px; margin: 0 auto; padding: 2em; background: #faf8f5; }
  h1 { font-family: Georgia, serif; font-size: 24pt; margin-bottom: 0.3em; color: #0a0806; }
  .narrator { margin: 0 0 0.8em 0; }
  .user { font-style: italic; color: #666; margin: 0 0 0.8em 0; padding-left: 1em; border-left: 2px solid #d9ff72; }
  hr { border: none; border-top: 1px solid #ddd; margin: 2em 0; }
  figure { margin: 1.5em 0; text-align: center; }
  figcaption { margin-top: 0.3em; }
  @media print { body { font-size: 11pt; } }
</style>
</head>
<body>
<h1>${escapeHtml(title)}</h1>
${bodyHTML}
<p style="text-align:center;color:#999;font-size:0.8em;margin-top:3em;">✦ Generated with FPV</p>
</body>
</html>`;
    const blob = new Blob([html], { type: "text/html;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${title.replace(/[^a-zA-Z0-9]/g, "_")}.html`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    // Delay revoke — browser may fetch blob asynchronously
    setTimeout(() => URL.revokeObjectURL(url), 5000);
    toast("Story saved", {
      description: `"${title}.html" saved.`,
      duration: 3000,
    });
  }, [messages, currentWorld, sceneImages]);

  // ── Navigate back to library ─────────────────────────────────────
  const goBack = useCallback(() => {
    setActiveView("library");
  }, [setActiveView]);

  // ── Render ───────────────────────────────────────────────────────
  return (
    <div className="flex-1 flex flex-col h-full overflow-hidden bg-[var(--color-bg-content)]">
      {/* First-entry tutorial — shown once, only on empty session */}
      {!loading && messages.length === 0 && preferences["tutorial_shown"] !== "true" && (
        <TutorialOverlay />
      )}

      {/* ── Header ──────────────────────────────────────────────── */}
      <div className="flex items-center justify-between px-6 py-3 border-b border-[var(--color-separator)]/40 shrink-0 relative">
        <div className="flex items-center gap-2">
          <button
            onClick={goBack}
            className="inline-flex items-center gap-2 text-[13px] font-medium text-[var(--color-label-secondary)] hover:text-[var(--color-label-primary)] transition-colors"
          >
            <ArrowLeft className="w-4 h-4" />
            {t("ui.stories", "Stories")}
          </button>
         </div>
        <div className="flex items-center gap-3">
          {currentWorld && (
             <div className="flex items-center gap-2 max-w-[260px]">
               <span className="text-[12px] text-[var(--color-label-tertiary)] truncate min-w-0">
               {currentWorld.genre && <span className="uppercase tracking-wider text-[10px]">{currentWorld.genre}</span>}
               {currentWorld.genre && <span className="mx-1.5">·</span>}
               <span className="font-display text-[11px] tracking-[0.04em]">{currentWorld.name}</span>
               </span>
               <span className={`shrink-0 rounded-full px-2 py-0.5 text-[9px] uppercase tracking-wider ${currentWorld.privacy_mode === "cloud_allowed" ? "bg-amber-500/15 text-amber-300" : "bg-emerald-500/15 text-emerald-300"}`} title={currentWorld.privacy_mode === "cloud_allowed" ? "Cloud narration is allowed for this story" : "Cloud narration is blocked for this story"}>
                {currentWorld.privacy_mode === "cloud_allowed" ? "Cloud allowed" : "Local only"}
               </span>
             </div>
          )}
          {messages.length > 0 && (
            <div className="flex items-center gap-1">
              <button
                onClick={exportClipboard}
                className="p-1.5 rounded-lg text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)] transition-colors"
                title={t("ui.copy-story-as-markdown", "Copy story as markdown")}
                aria-label={t("ui.copy-story-as-markdown", "Copy story as markdown")}
              >
                <Download className="w-3.5 h-3.5" />
              </button>
              <button
                onClick={exportSaveFile}
                className="p-1.5 rounded-lg text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)] transition-colors"
                title={t("ui.save-as-html-file", "Save as HTML file")}
                aria-label={t("ui.save-story-as-html-file", "Save story as HTML file")}
              >
                <FileDown className="w-3.5 h-3.5" />
              </button>
              <button
                onClick={exportPrint}
                className="p-1.5 rounded-lg text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)] transition-colors"
                title={t("ui.open-print-pdf", "Open print/PDF")}
                aria-label={t("ui.open-print-or-pdf-preview", "Open print or PDF preview")}
              >
                <Printer className="w-3.5 h-3.5" />
              </button>
            </div>
          )}
        </div>
      </div>

      {/* --- Generation controls (collapsible) -------------------- */}
      <div className="border-b border-[var(--color-separator)]/40 shrink-0">
        <button
          onClick={() => setGenerationPanelOpen((v) => !v)}
          className="flex items-center justify-between w-full px-6 py-2 text-[11px] font-medium text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] transition-colors"
          aria-expanded={generationPanelOpen}
          aria-controls="generation-controls"
        >
          <span className="flex items-center gap-2">
            <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M10.5 6h9.75M10.5 6a1.5 1.5 0 11-3 0m3 0a1.5 1.5 0 10-3 0M3.75 6H7.5m3 12h9.75m-9.75 0a1.5 1.5 0 01-3 0m3 0a1.5 1.5 0 00-3 0m-3.75 0H7.5m9-6h3.75m-3.75 0a1.5 1.5 0 01-3 0m3 0a1.5 1.5 0 00-3 0m-9.75 0h9.75" />
            </svg>
            {t("ui.generation", "Generation")}
          </span>
          <ChevronDown className={`w-3.5 h-3.5 transition-transform ${generationPanelOpen ? "rotate-180" : ""}`} />
        </button>
        {generationPanelOpen && (
          <div id="generation-controls" className="px-6 pb-4 space-y-3">
            <div>
              <div className="text-[11px] text-[var(--color-label-secondary)] mb-1.5">{t("ui.performance-profile", "Performance profile")}</div>
              <div className="grid grid-cols-3 gap-1.5">
                {(["efficient", "balanced", "quality"] as const).map((profile) => (
                  <button
                    key={profile}
                    onClick={() => applyPerformanceProfile(profile)}
                    className={`rounded-lg px-2 py-1.5 text-[10px] font-medium capitalize transition-colors ${performanceProfile === profile ? "bg-[var(--color-accent)] text-black" : "bg-[var(--color-fill-quaternary)] text-[var(--color-label-secondary)] hover:text-[var(--color-label-primary)]"}`}
                  >
                    {profile}
                  </button>
                ))}
              </div>
            </div>
            <label className="flex items-center justify-between rounded-lg border border-[var(--color-separator)]/30 px-2.5 py-2 text-[11px] text-[var(--color-label-secondary)]">
              <span><strong className="text-[var(--color-label-primary)]">{t("ui.advanced-controls", "Advanced controls")}</strong><span className="block text-[10px] text-[var(--color-label-tertiary)]">{t("ui.context-sampling-memory-and-scene-settings", "Context, sampling, memory, and scene settings.")}</span></span>
              <input type="checkbox" checked={advancedControls} onChange={(event) => updatePreference("generation_advanced", String(event.target.checked))} className="accent-[var(--color-accent)]" aria-label={t("ui.enable-advanced-generation-controls", "Enable advanced generation controls")} />
            </label>
            {advancedControls && <>
            {/* Unpredictability */}
            <div>
              <div className="flex justify-between text-[11px] text-[var(--color-label-secondary)] mb-0.5">
                <span>{t("ui.unpredictability", "Unpredictability")}</span>
                <span className="font-medium text-[var(--color-label-primary)]">{genTemperature.toFixed(1)}</span>
              </div>
              <Slider
                value={genTemperature}
                onChange={(v) => updatePreference("generation_temperature", String(Math.round(v * 10) / 10))}
                min={0.1}
                max={2.0}
                step={0.1}
                leftLabel="0.1"
                rightLabel="2.0"
                ariaLabel="Unpredictability"
              />
            </div>
            {/* Vocabulary range */}
            <div>
              <div className="flex justify-between text-[11px] text-[var(--color-label-secondary)] mb-0.5">
                <span>{t("ui.vocabulary-range", "Vocabulary range")}</span>
                <span className="font-medium text-[var(--color-label-primary)]">{genTopP.toFixed(1)}</span>
              </div>
              <Slider
                value={genTopP}
                onChange={(v) => updatePreference("generation_top_p", String(Math.round(v * 10) / 10))}
                min={0.1}
                max={1.0}
                step={0.1}
                leftLabel="0.1"
                rightLabel="1.0"
                ariaLabel="Vocabulary range"
              />
            </div>
            {/* Response length */}
            <div>
              <div className="flex justify-between text-[11px] text-[var(--color-label-secondary)] mb-0.5">
                <span>{t("ui.response-length", "Response length")}</span>
                <span className="font-medium text-[var(--color-label-primary)]">{genMaxTokens}</span>
              </div>
              <Slider
                value={genMaxTokens}
                onChange={(v) => updatePreference("generation_max_tokens", String(Math.round(v / 64) * 64))}
                min={64}
                max={4096}
                step={64}
                leftLabel="64"
                rightLabel="4096"
                ariaLabel="Response length"
              />
            </div>
            <label className="flex items-center gap-2 text-[11px] text-[var(--color-label-secondary)]">
              <input
                type="checkbox"
                checked={autoSceneImages}
                onChange={(event) => updatePreference("story_image_frequency", event.target.checked ? "every" : "off")}
                className="accent-[var(--color-accent)]"
              />
              {t("ui.automatically-illustrate-new-scenes", "Automatically illustrate new scenes")}
            </label>
            <div>
              <div className="flex justify-between text-[11px] text-[var(--color-label-secondary)] mb-0.5"><span>{t("ui.context-window", "Context window")}</span><span className="font-medium text-[var(--color-label-primary)]">{preferences["contextSize"] || "8192"}</span></div>
              <Slider value={Number(preferences["contextSize"] || 8192)} onChange={(v) => updatePreference("contextSize", String(Math.round(v / 1024) * 1024))} min={4096} max={32768} step={1024} leftLabel="4K" rightLabel="32K" ariaLabel="Context window" />
            </div>
            <label className="flex items-center gap-2 text-[11px] text-[var(--color-label-secondary)]"><input type="checkbox" checked={preferences["semanticMemoryEnabled"] === "true"} onChange={(event) => updatePreference("semanticMemoryEnabled", String(event.target.checked))} className="accent-[var(--color-accent)]" /> {t("ui.use-semantic-memory-when-available", "Use semantic memory when available")}</label>
            <div className="border-t border-[var(--color-separator)]/30 pt-3"><div className="mb-1.5 text-[11px] font-medium text-[var(--color-label-secondary)]">{t("ui.saved-presets", "Saved presets")}</div><div className="flex gap-1.5"><input value={presetName} onChange={(event) => setPresetName(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") saveAdvancedPreset(); }} placeholder={t("ui.preset-name", "Preset name")} className="min-w-0 flex-1 rounded-lg bg-[var(--color-fill-tertiary)] px-2 py-1.5 text-[11px] text-[var(--color-label-primary)] outline-none" aria-label={t("ui.preset-name", "Preset name")} /><button onClick={saveAdvancedPreset} disabled={!presetName.trim()} className="rounded-lg bg-[var(--color-accent-soft)] px-2 text-[10px] text-[var(--color-accent)] disabled:opacity-40">{t("ui.save", "Save")}</button></div>{advancedPresets.length > 0 && <div className="mt-1.5 flex flex-wrap gap-1">{advancedPresets.map((preset) => <button key={preset.name} onClick={() => loadAdvancedPreset(preset)} className="rounded-md border border-[var(--color-separator)]/30 px-1.5 py-1 text-[10px] text-[var(--color-label-secondary)] hover:border-[var(--color-accent)]/50 hover:text-[var(--color-accent)]">{preset.name}</button>)}</div>}</div>
            </>}
          </div>
        )}
      </div>



      {/* ── Error banner ────────────────────────────────────────── */}
      {error && (
        <div className="mx-6 mt-4 px-4 py-3 rounded-xl bg-red-500/10 border border-red-500/20">
          <p className="text-[12px] text-[var(--color-system-red)]">{error}</p>
        </div>
      )}

      {/* ── Memory summary indicator ────────────────────────────── */}
      {memorySummary && (
        <details className="mx-6 mt-2 mb-0 group cursor-pointer">
          <summary className="text-[10px] text-[var(--color-label-tertiary)] hover:text-[var(--color-label-secondary)] transition-colors tracking-wider uppercase font-medium">
            {t("ui.story-so-far-2", "Story so far…")}
          </summary>
          <p className="text-[11px] text-[var(--color-label-tertiary)] mt-1 leading-relaxed pl-2 border-l border-[var(--color-separator)]/20">
            {memorySummary.slice(0, 300)}
            {memorySummary.length > 300 ? "…" : ""}
          </p>
        </details>
      )}

      {/* ── Message scroll area ─────────────────────────────────── */}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        role="region"
        aria-label={t("ui.story-narration", "Story narration")}
        className="flex-1 overflow-y-auto px-6 py-6 space-y-5"
      >
        <div role="log" aria-live="polite" aria-atomic="false" className="contents">
        {/* Loading spinner */}
        {loading && (
          <div className="flex items-center justify-center py-20">
            <Loader2 className="w-5 h-5 animate-spin text-[var(--color-label-tertiary)]" />
          </div>
        )}

        {/* Empty state */}
        {!loading && messages.length === 0 && !error && (
          <div className="text-center py-20 px-6">
            <div className="w-16 h-16 rounded-2xl bg-gradient-to-br from-[var(--color-accent)]/15 to-transparent mx-auto mb-5 flex items-center justify-center border border-[var(--color-accent)]/10">
              <svg className="w-7 h-7 text-[var(--color-accent)]/40" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 20.25c4.97 0 9-3.694 9-8.25s-4.03-8.25-9-8.25S3 7.444 3 12c0 2.104.859 4.023 2.273 5.48.432.447.74 1.04.586 1.641a4.483 4.483 0 01-.923 1.785A5.969 5.969 0 006 21c1.282 0 2.47-.402 3.445-1.087.81.22 1.668.337 2.555.337z" />
              </svg>
            </div>
            <p className="text-[14px] font-medium text-[var(--color-label-primary)]">
              {currentWorld ? `Entering ${currentWorld.name}…` : "This story hasn't begun yet."}
            </p>
            <p className="text-[12px] text-[var(--color-label-tertiary)] mt-2 max-w-xs mx-auto leading-relaxed">
              {t("ui.write-your-first-action-below-to-step-into-the-w", "Write your first action below to step into the world. Every story begins with a single choice.")}
            </p>
          </div>
        )}

        {/* No-session error */}
        {!loading && !currentSessionId && (
          <div className="text-center py-20">
            <p className="text-[14px] text-[var(--color-label-secondary)]">
              {error}
            </p>
          </div>
        )}

        {/* Message list — flowing text with scene separators */}
        {messages.map((msg, idx) => {
          const isUser = msg.role === "user";
          const prevRole = idx > 0 ? messages[idx - 1].role : null;
          const showSceneBreak = prevRole === "narrator" && isUser;

          return (
            <div key={msg.id} data-msg-idx={idx} className="story-message">
              {/* Scene break between consecutive narrator paragraphs */}
              {showSceneBreak && <OrnamentDivider className="my-6" />}
              {isUser ? (
                <div className="pl-5 border-l-2 border-[var(--color-separator)]/20 mb-5 group">
                  {editingMessageId === msg.id ? (
                    <div className="space-y-2">
                      <textarea
                        value={editingText}
                        onChange={(event) => setEditingText(event.target.value)}
                        rows={3}
                        autoFocus
                        className="w-full resize-y rounded-lg bg-[var(--color-fill-tertiary)] text-[var(--color-label-primary)] text-[13px] leading-relaxed px-3 py-2 outline-none border border-[var(--color-accent)]/50"
                        aria-label={t("ui.edit-player-action", "Edit player action")}
                      />
                      <div className="flex items-center gap-2">
                        <button onClick={saveMessageEdit} className="inline-flex items-center gap-1 text-[11px] text-[var(--color-accent)]"><Check className="w-3 h-3" /> {t("ui.save", "Save")}</button>
                        <button onClick={cancelMessageEdit} className="inline-flex items-center gap-1 text-[11px] text-[var(--color-label-tertiary)]"><X className="w-3 h-3" /> {t("ui.cancel", "Cancel")}</button>
                      </div>
                    </div>
                  ) : (
                    <p className="text-[var(--color-label-secondary)] italic text-[13px] leading-relaxed whitespace-pre-wrap">
                      {msg.content}
                    </p>
                  )}
                  {!generating && editingMessageId !== msg.id && (
                    <button
                      onClick={() => beginMessageEdit(msg)}
                       className="mt-1 mr-3 text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] transition-colors opacity-0 group-hover:opacity-100 group-focus-within:opacity-100"
                    >
                      {t("ui.edit", "Edit")}
                    </button>
                  )}
                  {!generating && idx === messages.length - 1 && idx > 0 && (
                    <button
                      onClick={() => {
                        // Optimistic local trim; the backend deletes the
                        // persisted turn and rewinds the story state (V29
                        // history). Reload afterwards to sync the truth.
                        setMessages((prev) => prev.slice(0, idx));
                        setInput(lastUserActionRef.current);
                        if (!currentSessionId) return;
                        invoke("session_undo", { session_id: currentSessionId })
                          .catch(() => {})
                          .finally(() => setReloadKey((k) => k + 1));
                      }}
                       className="mt-1 text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-label-secondary)] transition-colors opacity-0 group-hover:opacity-100 group-focus-within:opacity-100"
                    >
                      {t("ui.undo", "Undo")}
                    </button>
                  )}
                </div>
              ) : (
                <div className="mb-5 group">
                  {editingMessageId === msg.id ? (
                    <div className="space-y-2">
                      <textarea
                        value={editingText}
                        onChange={(event) => setEditingText(event.target.value)}
                        rows={6}
                        autoFocus
                        className="w-full resize-y rounded-lg bg-[var(--color-fill-tertiary)] text-[var(--color-label-primary)] text-[13px] leading-relaxed px-3 py-2 outline-none border border-[var(--color-accent)]/50"
                        aria-label={t("ui.edit-narration", "Edit narration")}
                      />
                      <div className="flex items-center gap-2">
                        <button onClick={saveMessageEdit} className="inline-flex items-center gap-1 text-[11px] text-[var(--color-accent)]"><Check className="w-3 h-3" /> {t("ui.save", "Save")}</button>
                        <button onClick={cancelMessageEdit} className="inline-flex items-center gap-1 text-[11px] text-[var(--color-label-tertiary)]"><X className="w-3 h-3" /> {t("ui.cancel", "Cancel")}</button>
                      </div>
                    </div>
                  ) : (
                    <p className={`text-[var(--color-label-primary)] leading-[1.85] whitespace-pre-wrap ${narrationFont === "serif" ? "font-serif" : "font-sans"}`}
                       style={{ fontSize }}>
                      {msg.content}
                    </p>
                  )}
                  {sceneImages[idx] && (
                    <div className="mt-3 rounded-xl overflow-hidden border border-[var(--color-separator)]/30 max-w-sm">
                      <img src={sceneImages[idx]} alt="Scene illustration" loading="lazy" decoding="async" className="w-full h-auto" />
                    </div>
                  )}
                  {generatingScene === idx && !sceneImages[idx] && (
                    <div className="mt-2 flex items-center gap-2">
                      <div className="w-32">
                        <div className="h-1 rounded-full bg-[var(--color-fill-quaternary)] overflow-hidden">
                          <div className={`h-full rounded-full bg-[var(--color-accent)] ${sceneProgress > 0 ? "" : "w-1/3 animate-[progress-slide_1.4s_ease-in-out_infinite]"}`} style={sceneProgress > 0 ? { width: `${sceneProgress}%` } : undefined} />
                        </div>
                        <span className="mt-1 block text-[9px] text-[var(--color-label-quaternary)]">
                          {sceneProgress > 0 ? `${sceneProgress}%${sceneElapsed > 1000 ? ` · ~${Math.max(0, Math.ceil((sceneElapsed / 1000) * (100 - sceneProgress) / sceneProgress))}s left` : ""}` : "Preparing…"}
                        </span>
                      </div>
                      <button onClick={() => void invoke("image_generation_cancel")} className="text-[10px] text-[var(--color-label-tertiary)] hover:text-red-400">{t("ui.cancel", "Cancel")}</button>
                    </div>
                  )}
                  {sceneErrors[idx] && !sceneImages[idx] && !generatingScene && (
                    <p className="mt-1 text-[10px] text-red-400/80">{sceneErrors[idx]}</p>
                  )}
                   <div className="flex items-center gap-2 mt-1.5 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 transition-opacity">
                    {!generating && editingMessageId !== msg.id && (
                      <button onClick={() => beginMessageEdit(msg)} className="inline-flex items-center gap-1 text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] transition-colors">
                        <Edit3 className="w-3 h-3" /> {t("ui.edit", "Edit")}
                      </button>
                    )}
                    {!generating && idx === messages.length - 1 && (
                       <button onClick={() => handleRegenerate()} className="text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] transition-colors">
                        {t("ui.regenerate", "Regenerate")}
                      </button>
                    )}
                    {messages[idx]?.role === "narrator" && (
                      <>
                        {sceneFailures[idx] && !sceneImages[idx] && !generatingScene && (
                          <button onClick={() => void handleGenerateScene(idx)} className="text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] transition-colors">
                            {t("ui.retry-illustration", "Retry illustration")}
                          </button>
                        )}
                        {sceneImages[idx] && sceneSeeds[idx] !== undefined && !generatingScene && (
                          <button onClick={() => void handleGenerateScene(idx, false, sceneSeeds[idx])} className="text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] transition-colors">
                            {t("ui.same-seed", "Same seed")}
                          </button>
                        )}
                        {sceneImages[idx] && !generatingScene && (
                          <button onClick={() => void handleGenerateScene(idx)} className="text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] transition-colors">
                            {t("ui.new-variation", "New variation")}
                          </button>
                        )}
                        <button
                          onClick={() => handleGenerateScene(idx)}
                          disabled={generatingScene !== null}
                          className="text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] transition-colors disabled:opacity-40"
                        >
                          {generatingScene === idx ? "Generating scene…" : "Illustrate scene"}
                        </button>
                        {!generatingScene && (
                          <select
                            value={sceneStyle}
                             onChange={(e) => {
                               const style = e.target.value as SceneImageStyle;
                               setSceneStyle(style);
                               updatePreference("story_image_style", style);
                             }}
                            className="text-[10px] bg-transparent text-[var(--color-label-tertiary)] border border-[var(--color-separator)]/30 rounded px-1 py-0.5 outline-none hover:text-[var(--color-accent)] transition-colors"
                            aria-label={t("ui.image-style", "Image style")}
                          >
                            {SCENE_STYLES.map((s) => (
                              <option key={s} value={s} className="bg-[var(--color-bg-elevated)] text-[var(--color-label-primary)]">
                                {s}
                              </option>
                            ))}
                          </select>
                        )}
                      </>
                    )}
                  </div>
                </div>
              )}
            </div>
          );
        })}

        {/* Streaming text — appears as the AI writes */}
        {streamingText && (
          <div className="mb-5">
            <p className={`text-[var(--color-label-primary)] leading-[1.85] whitespace-pre-wrap ${narrationFont === "serif" ? "font-serif" : "font-sans"}`} style={{ fontSize }}>
              {streamingText}
              <span className="inline-block w-1 h-4 ml-0.5 bg-[var(--color-accent)]/60 animate-pulse" />
            </p>
            <button onClick={stopNarration} className="mt-2 text-[11px] text-red-400 hover:text-red-300">{t("ui.stop-generation", "Stop generation")}</button>
          </div>
        )}

        {/* Typing indicator — three bouncing dots */}
        {generating && !streamingText && (
          <div className="flex items-center gap-2.5 text-[var(--color-label-tertiary)] pb-4">
            <div className="flex items-center gap-1">
              <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-accent)]/60 animate-bounce" style={{ animationDelay: "0ms" }} />
              <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-accent)]/60 animate-bounce" style={{ animationDelay: "150ms" }} />
              <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-accent)]/60 animate-bounce" style={{ animationDelay: "300ms" }} />
            </div>
            <span className="text-[11px] italic">{t("ui.the-story-continues", "The story continues…")}</span>
            <button onClick={stopNarration} className="text-[11px] text-red-400 hover:text-red-300">{t("ui.stop", "Stop")}</button>
          </div>
        )}
        </div>
      </div>

      {!isNearBottom && messages.length > 0 && (
        <div className="flex justify-center -mb-2 relative z-10">
          <button
            onClick={() => scrollToBottom()}
            className="px-4 py-1.5 rounded-full bg-[var(--color-accent)] text-black text-[11px] font-semibold shadow-lg hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] transition-colors"
          >
            ↓ Scroll to bottom
          </button>
        </div>
      )}
      {/* ── Input area ──────────────────────────────────────────── */}
      <div className="shrink-0 px-6 py-4 border-t border-[var(--color-separator)]/40">
        {!generating && mode !== "visualize" && (
          <div className={`mb-2 flex items-center gap-2 text-[10px] ${narratorStatus?.chat_model_present ? "text-emerald-400" : "text-[var(--color-warm)]"}`}>
            <span className={`h-1.5 w-1.5 rounded-full ${narratorStatus?.chat_model_present ? "bg-emerald-400" : "bg-[var(--color-warm)]"}`} />
            {narratorStatus === null ? "Checking narrator..." : narratorStatus.chat_model_present ? "Narrator ready" : narratorStatus.ollama_up ? "Choose or install a narrator model in Settings" : "Starting local narrator..."}
          </div>
        )}
        {(storyState?.pending_changes?.length ?? 0) > 0 && (
          <button onClick={() => useApp.getState().setRightPanelOpen(true)} className="mb-2 inline-flex items-center gap-1.5 rounded-lg border border-[var(--color-accent)]/30 bg-[var(--color-accent-soft)] px-2 py-1 text-[10px] text-[var(--color-accent)] hover:bg-[var(--color-accent)]/15">
            Review {storyState?.pending_changes?.length} proposed {storyState?.pending_changes?.length === 1 ? "change" : "changes"}
          </button>
        )}
        <div className="flex items-center gap-1 mb-2" role="radiogroup" aria-label={t("ui.input-mode", "Input mode")}>
          {(["do", "say", "story", "visualize"] as const).map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => setMode(m)}
              disabled={generating || generatingScene !== null}
              aria-pressed={mode === m}
              className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-[11px] font-medium transition-all ${
                mode === m
                  ? "bg-[var(--color-accent)] text-black"
                  : "text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)]"
              } disabled:opacity-40`}
            >
              {m === "do" ? <PenLine className="w-3 h-3" /> : m === "say" ? <Quote className="w-3 h-3" /> : m === "story" ? <MessageSquare className="w-3 h-3" /> : <ImageIcon className="w-3 h-3" />}
              {m === "do" ? "Do" : m === "say" ? "Say" : m === "story" ? "Story" : "Visualize"}
            </button>
          ))}
        </div>
        {messages.length > 0 && messages[messages.length - 1].role === "narrator" && !generating && (
          <div className="flex items-center gap-2 mb-2">
            <button onClick={handleContinue} className="px-2.5 py-1 rounded-lg bg-[var(--color-accent)]/15 text-[11px] text-[var(--color-accent)] hover:bg-[var(--color-accent)]/25">{t("ui.continue", "Continue")}</button>
            <button onClick={() => handleRegenerate("make the scene more tense, while preserving the established facts") } className="text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)]">{t("ui.more-tension", "More tension")}</button>
            <button onClick={() => handleRegenerate("use more natural dialogue and subtext") } className="text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)]">{t("ui.more-dialogue", "More dialogue")}</button>
            <button onClick={() => handleRegenerate("slow the pacing and deepen sensory detail") } className="text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)]">{t("ui.slower", "Slower")}</button>
          </div>
        )}
        {generatingScene === -1 && (
          <div className="mb-3 flex items-center gap-2 text-[11px] text-[var(--color-label-tertiary)]">
            <span>Generating visualization {sceneProgress > 0 ? `${sceneProgress}%` : "..."}</span>
            <button onClick={() => void invoke("image_generation_cancel")} className="text-red-400 hover:text-red-300">{t("ui.cancel", "Cancel")}</button>
          </div>
        )}
        {visualizedImage && (
          <div className="mb-3 max-w-sm overflow-hidden rounded-xl border border-[var(--color-separator)]/30">
            <img src={visualizedImage} alt="Generated visualization" className="w-full h-auto" />
          </div>
        )}
        <div className="flex items-end gap-3">
          <textarea
            ref={inputRef}
            onFocus={prewarmNarrator}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            aria-label={t("ui.your-action", "Your action")}
            placeholder={
              mode === "visualize"
                ? "Describe what you want to visualize..."
                : mode === "do"
                ? currentWorld ? `What happens in ${currentWorld.name}?` : "What do you do?"
                : mode === "say"
                  ? '"What would you like to say?"'
                  : "Describe the scene..."
            }
            rows={2}
            disabled={generating || generatingScene !== null || !currentWorldId}
            className="flex-1 resize-none rounded-xl bg-[var(--color-fill-tertiary)] text-[var(--color-label-primary)] placeholder:text-[var(--color-label-tertiary)] text-[13px] leading-relaxed px-4 py-3 outline-none border border-transparent focus:border-[var(--color-accent)]/50 transition-colors disabled:opacity-40"
          />
          <button
             onClick={() => void handleSubmit()}
             disabled={!input.trim() || generating || generatingScene !== null || !currentWorldId}
            className="p-3 rounded-xl bg-[var(--color-accent)] hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] text-black transition-colors disabled:opacity-40 disabled:cursor-not-allowed shrink-0"
            aria-label={t("ui.submit-action", "Submit action")}
          >
            {generating ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <Send className="w-4 h-4" />
            )}
          </button>
        </div>
      </div>

    </div>
  );
}
