#!/usr/bin/env node
// One-time export of FPV mobile's community + preset worlds from Supabase
// into the FPV2.0 seed format (`src-tauri/resources/seed-data/worlds-community.json`).
// Also downloads each world's cover into `src-tauri/resources/seed-images/covers/`.
//
// The supabase keys are taken from the environment or `scripts/.supabase-secret`
// (gitignored). Run:
//   node scripts/export-supabase-worlds.mjs
//
// This is a build-time curator script — the keys are NOT part of the shipped
// app. FPV2.0 stays local-first; this is a one-shot data migration.

import { mkdirSync, writeFileSync, readFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, "..");

const SUPABASE_URL =
  process.env.SUPABASE_URL || "https://uyddkzpiiplzltakkblu.supabase.co";

// Read the service key from env, else from the local (gitignored) key file.
let SERVICE_KEY =
  process.env.SUPABASE_SERVICE_KEY || process.env.SUPABASE_SECRET_KEY;
const KEY_FILE = join(__dirname, ".supabase-secret");
if (!SERVICE_KEY && existsSync(KEY_FILE)) {
  SERVICE_KEY = readFileSync(KEY_FILE, "utf8").trim();
}

if (!SERVICE_KEY) {
  console.error("Missing SUPABASE_SERVICE_KEY / SUPABASE_SECRET_KEY");
  process.exit(1);
}

const REST = `${SUPABASE_URL}/rest/v1/worlds`;
const STORAGE = `${SUPABASE_URL}/storage/v1/object/public/world-covers`;

async function fetchWorlds() {
  const url = `${REST}?select=id,name,genre,description,system_prompt,is_preset,is_published,cover_image_url,like_count&or=(is_published.eq.true,is_preset.eq.true)`;
  const res = await fetch(url, {
    headers: {
      apikey: SERVICE_KEY,
      Authorization: `Bearer ${SERVICE_KEY}`,
    },
  });
  if (!res.ok) throw new Error(`worlds fetch failed: ${res.status} ${await res.text()}`);
  return res.json();
}

async function downloadCover(url, filename) {
  const dir = join(ROOT, "src-tauri/resources/seed-images/covers");
  mkdirSync(dir, { recursive: true });
  const dest = join(dir, filename);
  const res = await fetch(url);
  if (!res.ok) throw new Error(`cover fetch failed: ${res.status} ${url}`);
  const buf = Buffer.from(await res.arrayBuffer());
  writeFileSync(dest, buf);
  console.log(`  [cover] ${filename} (${(buf.length / 1024).toFixed(0)} KB)`);
}

const worlds = await fetchWorlds();
console.log(`Fetched ${worlds.length} worlds from Supabase`);

const out = [];
let covers = 0;
for (const w of worlds) {
  let cover_image_path = null;
  if (w.cover_image_url) {
    // Public bucket URL → download to seed-images/covers/<slug>.<ext>
    const extMatch = w.cover_image_url.match(/\.(png|jpe?g|webp)(\?|$)/i);
    const ext = extMatch ? extMatch[1].toLowerCase().replace("jpeg", "jpg") : "png";
    const slug = (w.name || `world-${w.id}`).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
    const filename = `${slug}.${ext}`;
    try {
      const url = w.cover_image_url.startsWith("http")
        ? w.cover_image_url
        : `${STORAGE}/${w.cover_image_url.replace(/^\//, "")}`;
      await downloadCover(url, filename);
      cover_image_path = `covers/${filename}`;
      covers++;
    } catch (e) {
      console.warn(`  [skip] cover for "${w.name}": ${e.message}`);
    }
  }

  out.push({
    name: w.name,
    genre: ["fantasy", "scifi", "horror", "manga", "romance", "custom"].includes(w.genre)
      ? w.genre
      : "custom",
    description: w.description ?? "",
    system_prompt: w.system_prompt ?? "",
    accent_color: null, // Supabase FPV has no accent_color column — UI falls back to genre default
    is_nsfw: false,
    cover_image_path,
    source: "seed",
  });
}

const outPath = join(ROOT, "src-tauri/resources/seed-data/worlds-community.json");
writeFileSync(outPath, JSON.stringify(out, null, 2));
console.log(`\nWrote ${out.length} worlds to seed-data/worlds-community.json`);
console.log(`Downloaded ${covers} covers`);
