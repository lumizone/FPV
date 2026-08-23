import { describe, it, expect } from 'vitest';
import { limitNarrationWords, matchCodex, postProcess, estimateTokens } from '../orchestrator';
import { buildPrompt } from '../promptBuilder';
import type { CodexEntry, Message } from '../orchestrator';

// ---------------------------------------------------------------------------
// matchCodex
// ---------------------------------------------------------------------------

describe('matchCodex', () => {
  const entries: CodexEntry[] = [
    { id: '1', title: 'Sword of Dawn', content: 'A legendary blade forged in sunlight.', triggers: ['sword', 'dawn', 'legendary blade'] },
    { id: '2', title: 'Shadow Dagger', content: 'A concealed blade that drinks light.', triggers: ['dagger', 'shadow'] },
    { id: '3', title: 'Elixir of Life', content: 'A glowing potion that heals any wound.', triggers: ['elixir', 'potion', 'healing'] },
  ];

  it('returns matched entries whose triggers appear in recentText (case-insensitive)', () => {
    const result = matchCodex(entries, 'I draw my sword at dawn.');
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe('1');
  });

  it('excludes entries that have no trigger match in recentText', () => {
    const result = matchCodex(entries, 'I walk through the forest.');
    expect(result).toHaveLength(0);
  });

  it('matches case-insensitively', () => {
    const result = matchCodex(entries, 'ELIXIR OF LIFE');
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe('3');
  });

  it('returns empty array for empty entries', () => {
    const result = matchCodex([], 'anything');
    expect(result).toEqual([]);
  });

  it('returns empty array for null/undefined entries', () => {
    const result = matchCodex(null as unknown as CodexEntry[], 'anything');
    expect(result).toEqual([]);
  });
});

describe('matchCodex — caps and specificity', () => {
  const makeEntry = (id: string, triggers: string[]): CodexEntry => ({
    id,
    title: `Entry ${id}`,
    content: `Content for entry ${id}`,
    triggers,
  });

  const sixEntries = [
    makeEntry('A', ['aa']),
    makeEntry('B', ['b', 'bb']),
    makeEntry('C', ['cc', 'ccc']),
    makeEntry('D', ['dd', 'dddd']),
    makeEntry('E', ['e', 'eeeee']),
    makeEntry('F', ['ff']),
  ];

  it('caps at 5 entries', () => {
    const text = 'aa bb cc dd e ff';
    const result = matchCodex(sixEntries, text);
    expect(result.length).toBeLessThanOrEqual(5);
    expect(result).toHaveLength(5);
  });

  it('sorts by specificity (longest trigger match first)', () => {
    const text = 'ccc aa bb';
    const customEntries = [
      makeEntry('short', ['aa']),
      makeEntry('medium', ['bb']),
      makeEntry('long', ['ccc']),
    ];
    const result = matchCodex(customEntries, text);
    // 'ccc' (3 chars) should beat the equal-length 'aa' and 'bb' matches.
    expect(result[0].id).toBe('long');
    // Equal-length triggers retain source order after the longest match.
    expect(result[1].id).toBe('short');
    expect(result[2].id).toBe('medium');
  });
});

// ---------------------------------------------------------------------------
// buildPrompt
// ---------------------------------------------------------------------------

describe('buildPrompt', () => {
  const systemPrompt = 'You are a narrator in a fantasy world.';
  const summary = 'The hero entered the dark cave.';
  const codex: CodexEntry[] = [
    { id: '1', title: 'Crystal Shard', content: 'A glowing crystal that reveals hidden paths.', triggers: ['crystal'] },
  ];
  const recentMessages: Message[] = [
    { role: 'narrator', content: 'The cave walls glisten with moisture.', created_at: '2024-01-01T00:00:00Z' },
    { role: 'user', content: 'I examine the walls.', created_at: '2024-01-01T00:00:01Z' },
  ];
  const userAction = 'I touch the crystal.';

  it('includes the system prompt section', () => {
    const result = buildPrompt({ systemPrompt, summary, matchedCodex: codex, recentMessages, userAction });
    expect(result).toContain('[NARRATOR INSTRUCTIONS]');
    expect(result).toContain('You are a narrator in a fantasy world.');
  });

  it('includes the summary section', () => {
    const result = buildPrompt({ systemPrompt, summary, matchedCodex: codex, recentMessages, userAction });
    expect(result).toContain('[STORY SO FAR]');
    expect(result).toContain('The hero entered the dark cave.');
  });

  it('includes the codex/lore section', () => {
    const result = buildPrompt({ systemPrompt, summary, matchedCodex: codex, recentMessages, userAction });
    expect(result).toContain('[KNOWN ENTITIES — LORE]');
    expect(result).toContain('Crystal Shard: A glowing crystal that reveals hidden paths.');
  });

  it('includes the recent exchanges section', () => {
    const result = buildPrompt({ systemPrompt, summary, matchedCodex: codex, recentMessages, userAction });
    expect(result).toContain('[RECENT EXCHANGES]');
    expect(result).toContain('The cave walls glisten with moisture.');
    expect(result).toContain('I examine the walls.');
  });

  it('includes the player action section', () => {
    const result = buildPrompt({ systemPrompt, summary, matchedCodex: codex, recentMessages, userAction });
    expect(result).toContain('[PLAYER ACTION]');
    expect(result).toContain('I touch the crystal.');
  });

  it('produces sections in the correct order', () => {
    const result = buildPrompt({ systemPrompt, summary, matchedCodex: codex, recentMessages, userAction });
    const narrIdx = result.indexOf('[NARRATOR INSTRUCTIONS]');
    const storyIdx = result.indexOf('[STORY SO FAR]');
    const loreIdx = result.indexOf('[KNOWN ENTITIES — LORE]');
    const recentIdx = result.indexOf('[RECENT EXCHANGES]');
    const actionIdx = result.indexOf('[PLAYER ACTION]');

    expect(narrIdx).toBeLessThan(storyIdx);
    expect(storyIdx).toBeLessThan(loreIdx);
    expect(loreIdx).toBeLessThan(recentIdx);
    expect(recentIdx).toBeLessThan(actionIdx);
  });

  it('skips empty sections gracefully', () => {
    const result = buildPrompt({
      systemPrompt,
      summary: '',
      matchedCodex: [],
      recentMessages: [],
      userAction,
    });
    expect(result).not.toContain('[STORY SO FAR]');
    expect(result).not.toContain('[KNOWN ENTITIES — LORE]');
    expect(result).not.toContain('[RECENT EXCHANGES]');
    expect(result).toContain('[NARRATOR INSTRUCTIONS]');
    expect(result).toContain('[PLAYER ACTION]');
  });

  it('prefixes user messages with ">" in the history', () => {
    const result = buildPrompt({ systemPrompt, summary, matchedCodex: codex, recentMessages, userAction });
    const exchangesSection = result.split('[RECENT EXCHANGES]')[1].split('[PLAYER ACTION]')[0];
    expect(exchangesSection).toContain('> I examine the walls.');
    expect(exchangesSection).toContain('The cave walls glisten with moisture.');
  });
});

// ---------------------------------------------------------------------------
// postProcess
// ---------------------------------------------------------------------------

describe('postProcess', () => {
  it('strips **bold** markdown', () => {
    expect(postProcess('This is **bold** text.')).toBe('This is bold text.');
  });

  it('strips *italic* markdown', () => {
    expect(postProcess('This is *italic* text.')).toBe('This is italic text.');
  });

  it('strips _italic_ markdown', () => {
    expect(postProcess('This is _italic_ text.')).toBe('This is italic text.');
  });

  it('strips `code` backticks', () => {
    expect(postProcess('This is `code` here.')).toBe('This is code here.');
  });

  it('replaces rhythmically with steadily', () => {
    expect(postProcess('He breathed rhythmically.')).toBe('He breathed steadily.');
  });

  it('replaces cacophony with clatter', () => {
    expect(postProcess('A cacophony of noise.')).toBe('A clatter of noise.');
  });

  it('replaces visceral with raw', () => {
    expect(postProcess('A visceral feeling.')).toBe('A raw feeling.');
  });

  it('replaces unnaturally cold', () => {
    expect(postProcess('The air was unnaturally cold.')).toBe('The air was cold enough to ache.');
  });

  it('replaces unnaturally still with too still', () => {
    expect(postProcess('The room was unnaturally still.')).toBe('The room was too still.');
  });

  it('applies all transformations together', () => {
    const result = postProcess('**Bold** and *italic* moved `rhythmically` with a **visceral** *cacophony*.');
    expect(result).not.toContain('**');
    expect(result).not.toContain('*');
    expect(result).not.toContain('`');
    expect(result).not.toContain('rhythmically');
    expect(result).not.toContain('visceral');
    expect(result).not.toContain('cacophony');
  });

  it('strips markdown headings', () => {
    expect(postProcess('# Heading\n## Subheading')).toBe('Heading\nSubheading');
  });
});

describe('limitNarrationWords', () => {
  it('keeps output under the cap at a sentence boundary', () => {
    const result = limitNarrationWords('One short sentence. Two short sentences follow. Three should be removed.', 6);
    expect(result).toBe('One short sentence. Two short sentences');
    expect(result.split(/\s+/)).toHaveLength(6);
  });

  it('does not alter output already within the cap', () => {
    expect(limitNarrationWords('A complete scene.', 20)).toBe('A complete scene.');
  });
});

// ---------------------------------------------------------------------------
// estimateTokens
// ---------------------------------------------------------------------------

describe('estimateTokens', () => {
  it('returns Math.ceil(text.length / 4)', () => {
    expect(estimateTokens('hello')).toBe(2);   // 5/4 = 1.25 → 2
    expect(estimateTokens('abcd')).toBe(1);     // 4/4 = 1 → 1
    expect(estimateTokens('')).toBe(0);         // 0/4 = 0 → 0
    expect(estimateTokens('abcde')).toBe(2);    // 5/4 = 1.25 → 2
    expect(estimateTokens('a')).toBe(1);        // 1/4 = 0.25 → 1
    expect(estimateTokens('abcdefgh')).toBe(2); // 8/4 = 2 → 2
  });
});
