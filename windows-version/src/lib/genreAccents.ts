/**
 * Genre → accent color + glyph map, carried over from the FPV mobile app
 * (`constants/genres.ts`). Gives every story card a distinct accent color so
 * the library reads as a coherent palette, not a single gold wash.
 */

export const GENRE_ACCENTS: Record<string, string> = {
  fantasy: "#C9A84C",
  scifi: "#4A9ECC",
  horror: "#CC4A4A",
  manga: "#9B4ACC",
  romance: "#CC4A7A",
  custom: "#888888",
  erotica: "#F48FB1",
  action: "#E25563",
  adventure: "#E0A33E",
  cyberpunk: "#33C4B3",
  isekai: "#6C8BE0",
  mystery: "#7E8AA2",
  crime: "#8A6D5B",
  drama: "#C77D9E",
  comedy: "#E3B23C",
  "boys-love": "#6FA8DC",
  "girls-love": "#D98AB5",
  "slice-of-life": "#8FB996",
  historical: "#B08D57",
  wuxia: "#C77B47",
  sport: "#5BA86F",
  "post-apocalyptic": "#9A8767",
};

export const GENRE_GLYPHS: Record<string, string> = {
  fantasy: "⚔",
  scifi: "◎",
  horror: "†",
  manga: "⊛",
  romance: "♦",
  custom: "◈",
  erotica: "♥",
};

/// Accent color for a genre, falling back to the brand gold.
export function genreAccent(genre: string): string {
  return GENRE_ACCENTS[normalizeGenre(genre)] ?? "var(--color-accent)";
}

/// Glyph (or a compact fallback) for a genre.
export function genreGlyph(genre: string): string {
  return GENRE_GLYPHS[normalizeGenre(genre)] ?? "✦";
}

/** Legacy worlds created before the genre rename used "sci_fi"; the accent
 *  and glyph tables are keyed by "scifi". Normalize so both spellings get
 *  the same accent. */
function normalizeGenre(genre: string): string {
  const lower = genre.toLowerCase();
  return lower === "sci_fi" ? "scifi" : lower;
}
