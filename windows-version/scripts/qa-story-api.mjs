#!/usr/bin/env node
// Real-model story QA. This bypasses the renderer and exercises the same
// prompt builder and Ollama connection used by the app, making failures
// reproducible without relying on macOS WebView automation.

import { DatabaseSync } from "node:sqlite";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { buildPrompt, buildSystemFor, getNarrationBudget, OPENING_INSTRUCTION } from "../src/lib/narration/promptBuilder.ts";
import { limitNarrationWords, matchCodex, postProcess } from "../src/lib/narration/orchestrator.ts";

const root = join(import.meta.dirname, "..");
const dbPath = process.env.FPV_DB ?? join(process.env.HOME, "Library/Application Support/com.lumizone.fpvdesktop/app.db");
const ollamaUrl = process.env.OLLAMA_URL ?? "http://127.0.0.1:11434";
const model = process.env.FPV_QA_MODEL ?? "qwen3.5:9b";
const turns = Number(process.env.FPV_QA_TURNS ?? 8);
const worldName = process.env.FPV_QA_WORLD ?? "Ashwick County";

const actions = [
  "I examine the dark syrup and ask Mrs. Gable why the radio is speaking.",
  "I refuse the ladle and ask what happened to the people who disappeared this week.",
  "I search the kitchen for something that proves the house is not what it seems.",
  "I play the radio recording backwards and write down every name I hear.",
  "I leave the house and follow the smell of wet soil toward the abandoned water tower.",
  "I confront the sheriff at the water tower and demand the truth about the missing people.",
  "I enter the tunnel beneath the tower with the flashlight and keep the names in mind.",
  "I touch the sealed door and listen for whoever is breathing on the other side.",
];

function queryOne(db, sql, ...params) {
  return db.prepare(sql).get(...params);
}

function queryAll(db, sql, ...params) {
  return db.prepare(sql).all(...params);
}

async function chat(prompt, options = {}) {
  const response = await fetch(`${ollamaUrl}/api/chat`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      model,
      messages: [{ role: "user", content: prompt }],
      stream: false,
      think: false,
      options: { num_ctx: 8192, temperature: 0.85, top_p: 0.95, num_predict: 2048, ...options },
    }),
  });
  if (!response.ok) throw new Error(`Ollama ${response.status}: ${await response.text()}`);
  const body = await response.json();
  const content = body.message?.content ?? "";
  if (!content.trim()) throw new Error("Ollama returned an empty response");
  return content;
}

function wordCount(text) {
  return text.trim().split(/\s+/).filter(Boolean).length;
}

function sentenceCount(text) {
  return (text.match(/[.!?]+(?=\s|$)/g) ?? []).length;
}

function repeatedNgrams(text, size = 3) {
  const words = text.toLowerCase().replace(/[^\p{L}\p{N}\s]/gu, "").split(/\s+/).filter(Boolean);
  const counts = new Map();
  for (let i = 0; i <= words.length - size; i += 1) {
    const gram = words.slice(i, i + size).join(" ");
    counts.set(gram, (counts.get(gram) ?? 0) + 1);
  }
  return [...counts.entries()].filter(([, count]) => count > 1).sort((a, b) => b[1] - a[1]).slice(0, 5);
}

function parseState(raw, previous, narration) {
  const match = raw.match(/\{[\s\S]*\}/);
  if (!match) return { state: { ...previous, last_scene: narration.slice(-300) }, valid: false, raw };
  try {
    const value = JSON.parse(match[0]);
    const fields = ["active_goals", "open_threads", "inventory", "relationships", "facts", "conflicts"];
    const state = { ...previous, ...value, source: "model", last_scene: value.last_scene || narration.slice(-300) };
    for (const field of fields) {
      const extracted = Array.isArray(value[field]) ? value[field] : [];
      const previousItems = Array.isArray(previous[field]) ? previous[field] : [];
      state[field] = [
        ...previousItems,
        ...extracted.filter((item) => !previousItems.some((oldItem) => String(oldItem).toLowerCase() === String(item).toLowerCase())),
      ].slice(0, 8);
    }
    return { state, valid: true, raw };
  } catch {
    return { state: { ...previous, last_scene: narration.slice(-300) }, valid: false, raw };
  }
}

async function extractState(previous, action, narration) {
  const prompt = `You maintain continuity for an interactive story. Return ONLY valid JSON.\n\nExtract only confirmed facts from the previous state, player action, and current scene. Preserve existing entries. Keep arrays short.\n\nJSON schema: {"turn": number, "location": string, "active_goals": [""], "open_threads": [""], "inventory": [""], "relationships": [""], "facts": [""], "conflicts": [""], "last_scene": ""}\n\nPREVIOUS STATE:\n${JSON.stringify(previous)}\n\nPLAYER ACTION:\n${action}\n\nCURRENT SCENE:\n${narration}`;
  const raw = await chat(prompt, { temperature: 0.1, top_p: 0.9, num_predict: 512, format: "json" });
  return parseState(raw, previous, narration);
}

async function main() {
  const db = new DatabaseSync(dbPath, { readOnly: true });
  const world = queryOne(db, "SELECT name, genre, description, system_prompt FROM worlds WHERE name = ? LIMIT 1", worldName);
  if (!world) throw new Error(`World not found: ${worldName}`);
  const rawCodex = queryAll(db, "SELECT id, title, content, triggers FROM codex_entries WHERE world_id = (SELECT id FROM worlds WHERE name = ? LIMIT 1)", worldName);
  const codex = rawCodex.map((entry) => ({
    id: entry.id,
    title: entry.title,
    content: entry.content,
    triggers: JSON.parse(entry.triggers || "[]"),
  }));

  const messages = [];
  let state = { turn: 0, location: "", active_goals: [], open_threads: [], inventory: [], relationships: [], facts: [], conflicts: [], last_scene: "", source: "model" };
  const results = [];
  const started = Date.now();

  for (let index = 0; index < Math.min(turns, actions.length); index += 1) {
    const action = actions[index];
    const recentText = `${action} ${messages.slice(-2).map((message) => message.content).join(" ")}`;
    const matchedCodex = matchCodex(codex, recentText);
    const budget = getNarrationBudget("dramatic", action, index + 1);
    const prompt = buildPrompt({
      systemPrompt: buildSystemFor(world, { narrative_theme: "dark", narrative_style: "dramatic", narrative_freedom: "balanced" }, budget),
      summary: state.last_scene,
      matchedCodex,
      recentMessages: messages.slice(-8),
      userAction: action,
      turnNumber: index + 1,
      openingInstruction: index === 0 ? OPENING_INSTRUCTION : undefined,
      storyState: JSON.stringify(state),
    });
    const raw = await chat(prompt);
    const narration = limitNarrationWords(postProcess(raw), budget.hardCap);
    const stateResult = await extractState(state, action, narration);
    state = { ...stateResult.state, turn: index + 1 };
    messages.push({ role: "user", content: action, created_at: new Date().toISOString() });
    messages.push({ role: "narrator", content: narration, created_at: new Date().toISOString() });
    const stats = {
      turn: index + 1,
      words: wordCount(narration),
      target: `${budget.targetMin}-${budget.targetMax}`,
      hard_cap: budget.hardCap,
      chars: narration.length,
      sentences: sentenceCount(narration),
      repeated_ngrams: repeatedNgrams(narration),
      state_json_valid: stateResult.valid,
      state_location: state.location,
      state_facts: state.facts.length,
      state_threads: state.open_threads.length,
      codex_matches: matchedCodex.length,
      preview: narration.slice(0, 220).replace(/\s+/g, " "),
    };
    results.push(stats);
    console.log(JSON.stringify(stats));
  }

  const outputDir = process.env.QA_REPORT_DIR ?? join(root, "qa-reports");
  mkdirSync(outputDir, { recursive: true });
  const reportPath = join(outputDir, `story-api-${new Date().toISOString().replace(/[:.]/g, "-")}.md`);
  const averageWords = results.reduce((sum, result) => sum + result.words, 0) / Math.max(results.length, 1);
  const report = [
    "# FPV Real Story API QA",
    "",
    `- World: ${world.name}`,
    `- Model: ${model}`,
    `- Turns: ${results.length}`,
    `- Duration: ${((Date.now() - started) / 1000).toFixed(1)}s`,
    `- Average narration length: ${averageWords.toFixed(0)} words`,
    `- Valid Story State responses: ${results.filter((result) => result.state_json_valid).length}/${results.length}`,
    "",
    "## Turn Metrics",
    "",
    "| Turn | Words | Target | Cap | Chars | Sentences | State JSON | Location | Codex |",
    "|---:|---:|---:|---:|---:|---:|:---:|---|---:|",
    ...results.map((result) => `| ${result.turn} | ${result.words} | ${result.target} | ${result.hard_cap} | ${result.chars} | ${result.sentences} | ${result.state_json_valid ? "yes" : "NO"} | ${result.state_location || "(empty)"} | ${result.codex_matches} |`),
    "",
    "## Notes",
    "",
    "This test uses the production prompt builder and the real Ollama HTTP API, but bypasses Tauri IPC and the renderer. Use it for model/story quality and continuity; use `npm run qa` for application contracts and `npm run qa:gui` for launch smoke.",
  ].join("\n");
  writeFileSync(reportPath, `${report}\n`, "utf8");
  console.log(`REPORT=${reportPath}`);
}

main().catch((error) => {
  console.error(`STORY QA FAIL: ${error.message}`);
  process.exitCode = 1;
});
