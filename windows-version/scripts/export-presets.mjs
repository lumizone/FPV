#!/usr/bin/env node
/**
 * Export FPV's PRESET_WORLDS to seed-data/worlds-presets.json
 *
 * Strategy: find exact line ranges, extract only the array body,
 * strip TS-only syntax from property keys.
 */

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(__dirname, "..");

const FPV_WORLDS = join(
  REPO_ROOT,
  "..",
  "..",
  "FPV- project",
  "FPV-experiment-local",
  "app",
  "fpv",
  "constants",
  "worlds.ts",
);

const src = readFileSync(FPV_WORLDS, "utf-8");
const lines = src.split("\n");

// Find the line with PRESET_WORLDS declaration
let startLine = lines.findIndex((l) => l.includes("PRESET_WORLDS"));
if (startLine === -1) {
  console.error("Could not find PRESET_WORLDS");
  process.exit(1);
}

// Find the line before GENRE_LABELS
let endLine = lines.findIndex((l, i) => i > startLine && l.includes("GENRE_LABELS"));
if (endLine === -1) endLine = lines.length;

// Extract just the array lines (from opening [ to closing ])
const arrayLines = lines.slice(startLine, endLine);

// Remove the first line (const declaration), keep array body
// First line: `export const PRESET_WORLDS: PresetWorld[] = [`
// Remove everything up to and including the `= [`
arrayLines[0] = arrayLines[0].replace(/^.*?=\s*\[/, "[");

// Process each line: remove TS-only syntax on property keys
const cleaned = arrayLines
  .map((l) => {
    // Remove whole-line comments
    if (l.trim().startsWith("//")) return "";

    // Remove is_preset: true
    if (l.trim().startsWith("is_preset:")) return "";

    // Remove type annotations on property keys
    // Match: `key_name: TypeWord,` or `key_name: TypeWord`
    // Only match when it looks like a JS object property key
    // (after whitespace at line start or after {)
    let line = l;

    // Remove `: Genre` type annotation
    line = line.replace(/:\s*Genre\b/g, "");

    // Remove `: string` type annotation
    line = line.replace(/:\s*string\b/g, "");

    // Remove `: true` (standalone, not `: true,`)
    line = line.replace(/:\s*true\b(?!\s*$)/g, (m) => {
      // If followed by comma or nothing useful, strip
      return "";
    });

    return line;
  })
  .filter((l) => l.trim() !== "")
  .join("\n");

// Now handle the specific issue: `: true` in is_preset was already removed,
// but other `: true` patterns might remain. Also fix `: true` that's now dangling.
// The regex above strips `: true` as type annotation too.

// Build JS
const jsSrc = `(function() { return ${cleaned}; })()`;

let presets;
try {
  presets = eval(jsSrc);
} catch (e) {
  console.error("Failed to eval:", e.message);
  console.error("--- cleaned array start ---");
  console.error(cleaned.slice(0, 500));
  console.error("--- end ---");
  process.exit(1);
}

if (!Array.isArray(presets)) {
  console.error("Result is not an array:", typeof presets);
  process.exit(1);
}

const seed = presets.map((w) => ({
  name: w.name,
  genre: w.genre,
  description: w.description || null,
  system_prompt: w.system_prompt,
  accent_color: w.accent || null,
  is_nsfw: false,
  cover_image_path: null,
  source: "seed",
}));

const outDir = join(REPO_ROOT, "seed-data");
mkdirSync(outDir, { recursive: true });
const outPath = join(outDir, "worlds-presets.json");
writeFileSync(outPath, JSON.stringify(seed, null, 2));

console.log(`Exported ${seed.length} preset worlds to ${outPath}`);
