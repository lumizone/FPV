// Strip LW companion namespaces from all 7 locale files
const fs = require("fs");
const path = require("path");

const DIR = path.resolve(__dirname, "../src/i18n/locales");
const FILES = ["de.json", "en.json", "es.json", "ja.json", "ko.json", "pl.json", "zh.json"];

const LW_TOPLEVEL = ["memory", "gallery", "common", "growth", "labels"];

const LW_APP_KEYS = ["sidebar", "moods", "chat", "kb", "mini", "drift", "proactive", "call", "crisis", "dictation"];

const LW_SETTINGS_KEYS = ["hardware", "network", "security", "characters", "byok", "telegram", "mcp", "voice", "voicemodels", "you"];

const LW_DRAWER_KEYS = ["shortcuts", "hardware", "image_models", "tts_models", "mcp", "network", "byok", "telegram", "security", "privacy", "advanced", "characters", "growth", "lorebook", "voice"];

for (const file of FILES) {
  const fp = path.join(DIR, file);
  const data = JSON.parse(fs.readFileSync(fp, "utf-8"));

  // 1. Delete entire LW namespaces
  for (const ns of LW_TOPLEVEL) delete data[ns];

  // 2. Strip LW sections from app.*
  if (data.app) {
    for (const k of LW_APP_KEYS) delete data.app[k];
    // Fix trial banner desc (used in App.tsx)
    if (data.app.trial?.banner_desc) {
      data.app.trial.banner_desc =
        "Buy a license to continue — all your data stays on this Mac.";
    }
  }

  // 3. Fix paywall strings
  if (data.paywall) {
    if (data.paywall.subtitle_expired) {
      data.paywall.subtitle_expired =
        "Buy a one-time lifetime license — your stories and worlds stay on this computer.";
    }
    if (data.paywall.tier_lifetime_price) {
      data.paywall.tier_lifetime_price = "$10";
    }
    if (data.paywall.paste_desc) {
      data.paywall.paste_desc =
        "Paste it here. We'll validate your key and activate this device.";
    }
  }

  // 4. Fix settings.about — LW "About your companion" → "About FPV"
  if (data.settings?.about?.ai) {
    if (data.settings.about.ai.title?.includes("About your companion")) {
      data.settings.about.ai.title = "About FPV";
    }
    if (data.settings.about.ai.heading?.includes("She is")) {
      data.settings.about.ai.heading = "This is an AI";
    }
    if (data.settings.about.ai.body) {
      data.settings.about.ai.body = data.settings.about.ai.body
        .replace(/She is not a person,?/gi, "It is not a person,")
        .replace(/ she has no awareness/gi, " it has no awareness")
        .replace(/ she cannot give/gi, " it cannot give");
    }
  }

  // 5. Clean settings.drawer — keep only active tabs
  if (data.settings?.drawer) {
    const keep = {};
    for (const [k, v] of Object.entries(data.settings.drawer)) {
      if (!LW_DRAWER_KEYS.includes(k)) keep[k] = v;
    }
    // Fix tab labels
    if (keep.appearance === "Appearance & Chat") keep.appearance = "Appearance";
    if (keep.chat) keep.chat = "Narration & Display";
    data.settings.drawer = keep;
  }

  // 6. Fix settings.chat.reasoning — "Let her reason" → "Let the model reason"
  if (data.settings?.chat?.reasoning?.desc) {
    data.settings.chat.reasoning.desc = data.settings.chat.reasoning.desc
      .replace("Let her reason", "Let the model reason")
      .replace(/more considered, but slower\. Off is faster/, "more thorough, but slower. Off is faster");
  }

  // 7. Strip LW settings namespaces
  if (data.settings) {
    for (const k of LW_SETTINGS_KEYS) delete data.settings[k];
  }

  // 8. Check for LW_HA_URL/LW_HA_TOKEN in any remaining string
  // (should be gone after deleting mcp, but just in case)

  fs.writeFileSync(fp, JSON.stringify(data, null, 2) + "\n");
  console.log(`✓ ${file}`);
}

console.log("\nDone — 7 locale files cleaned.");
