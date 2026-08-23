/**
 * Narration orchestration — ported from FPV mobile
 * (`FPV- project/FPV-experiment-local/app/fpv/lib/localLLM/orchestrator.ts`).
 *
 * Owns Codex/lore matching, context trimming, post-processing (markdown strip +
 * verbal-tic filter), and token estimation. Pure and dependency-free so it is
 * fully unit-testable.
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** A codex/lore entry describing a story entity. */
export interface CodexEntry {
  id: string;
  title: string;
  content: string;
  triggers: string[];
}

/** A single turn in the conversation history. */
export interface Message {
  id?: string;
  role: 'narrator' | 'user';
  content: string;
  created_at: string;
}

// ---------------------------------------------------------------------------
// Token estimation
// ---------------------------------------------------------------------------

/** Rough token-count estimate: 1 token per 4 characters. */
export function estimateTokens(text: string): number {
  return Math.ceil(text.length / 4);
}

// ---------------------------------------------------------------------------
// Post-processing
// ---------------------------------------------------------------------------

/** Strip markdown formatting — bold, italic, code, list markers, headings. */
export function stripMarkdown(text: string): string {
  return text
    .replace(/^#{1,6}\s+/gm, '')
    .replace(/\*\*(.+?)\*\*/gs, '$1')
    .replace(/\*(.+?)\*/gs, '$1')
    .replace(/__(.+?)__/gs, '$1')
    .replace(/_(.+?)_/gs, '$1')
    .replace(/`([^`]+)`/g, '$1')
    .replace(/^\s*[-*+]\s+/gm, '')
    .replace(/^\s*\d+\.\s+/gm, '')
    .trim();
}

/** Replace over-used verbal tics with less distracting alternatives. */
export function cleanVerbalTics(text: string): string {
  return text
    .replace(/\brhythmically\b/gi, 'steadily')
    .replace(/\brhythmic\b/gi, 'steady')
    .replace(/\bunnaturally cold\b/gi, 'cold enough to ache')
    .replace(/\bunnaturally (still|quiet|silent)\b/gi, 'too $1')
    .replace(/\bcacophony\b/gi, 'clatter')
    .replace(/\bvisceral\b/gi, 'raw');
}

/** Final cleanup applied to the model's raw output before display/persist. */
export function postProcess(rawOutput: string): string {
  return cleanVerbalTics(stripMarkdown(rawOutput));
}

/** Keep model output within the selected style cap without cutting a sentence
 * in the middle when a model ignores the prompt's length contract. */
export function limitNarrationWords(text: string, maxWords: number): string {
  if (!Number.isFinite(maxWords) || maxWords < 1) return text.trim();
  const words = text.trim().split(/\s+/).filter(Boolean);
  if (words.length <= maxWords) return text.trim();
  const truncated = words.slice(0, Math.floor(maxWords)).join(' ');
  const sentenceEnds = [...truncated.matchAll(/[.!?](?=\s|$)/g)].map((match) => (match.index ?? 0) + 1);
  const lastSentenceEnd = sentenceEnds[sentenceEnds.length - 1];
  return lastSentenceEnd && lastSentenceEnd >= truncated.length * 0.65
    ? truncated.slice(0, lastSentenceEnd).trim()
    : truncated.trim();
}

// ---------------------------------------------------------------------------
// Codex matching (ported from FPV mobile)
// ---------------------------------------------------------------------------

const CODEX_MAX_ENTRIES = 5;

/**
 * Match codex entry triggers against `recentText` (case-insensitive) and return
 * matched entries sorted by specificity (longest trigger match wins), capped at
 * 5. Non-matching entries are excluded.
 */
export function matchCodex(entries: CodexEntry[], recentText: string): CodexEntry[] {
  if (!entries || entries.length === 0) return [];

  const recentLower = recentText.toLowerCase();

  const matched: { entry: CodexEntry; specificity: number }[] = [];
  for (const entry of entries) {
    const triggers = Array.isArray(entry.triggers) ? entry.triggers : [];
    let best = 0;
    for (const trig of triggers) {
      const t = (trig || '').toLowerCase().trim();
      if (!t) continue;
      const normalizedRecent = recentLower.replace(/[^\p{L}\p{N}]+/gu, " ").trim();
      const normalizedTrigger = t.replace(/[^\p{L}\p{N}]+/gu, " ").trim();
      if (` ${normalizedRecent} `.includes(` ${normalizedTrigger} `) && t.length > best) best = t.length;
    }
    if (best > 0) {
      matched.push({ entry, specificity: best });
    }
  }

  matched.sort((a, b) => b.specificity - a.specificity);
  return matched.slice(0, CODEX_MAX_ENTRIES).map((m) => m.entry);
}

// ---------------------------------------------------------------------------
// Context trimming
// ---------------------------------------------------------------------------

export interface TrimOptions {
  systemPrompt: string;
  summary: string | null;
  codexContext: string | null;
  userMessage: string;
  /** Tier context window (tokens). */
  contextTokenTarget: number;
  /** Minimum history kept even when the system block is large. */
  historyFloor: number;
}

/**
 * Trim history to fit the tier window, reserving room for system + summary +
 * codex + the user action. Walks newest→oldest, keeping as much tail as fits.
 */
export function trimContext(
  messages: Message[],
  opts: TrimOptions,
): Message[] {
  const sysToks =
    estimateTokens(opts.systemPrompt) +
    (opts.summary ? estimateTokens(opts.summary) : 0) +
    (opts.codexContext ? estimateTokens(opts.codexContext) : 0) +
    estimateTokens(opts.userMessage);
  const contextBudget = Math.max(opts.historyFloor, opts.contextTokenTarget - sysToks);

  const trimmed: Message[] = [];
  let tokenCount = 0;
  for (let i = messages.length - 1; i >= 0; i--) {
    const t = estimateTokens(messages[i].content);
    if (tokenCount + t > contextBudget) break;
    trimmed.unshift(messages[i]);
    tokenCount += t;
  }
  return trimmed;
}
