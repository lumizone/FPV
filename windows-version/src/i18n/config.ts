import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import LanguageDetector from "i18next-browser-languagedetector";

import en from "./locales/en.json";
import pl from "./locales/pl.json";
import zh from "./locales/zh.json";
import ko from "./locales/ko.json";
import ja from "./locales/ja.json";
import de from "./locales/de.json";
import es from "./locales/es.json";

// Languages the UI ships full translations for. A detected OS/browser locale
// only takes effect on first run (no saved pref). App.tsx re-applies the
// persisted `language` pref on every boot, and onboarding persists an explicit
// choice — so detection never overrides what the user actually picked.
export const SUPPORTED_LANGUAGES = ["en", "pl", "zh", "ko", "ja", "de", "es"] as const;

i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      en: { translation: en },
      pl: { translation: pl },
      zh: { translation: zh },
      ko: { translation: ko },
      ja: { translation: ja },
      de: { translation: de },
      es: { translation: es },
    },
    supportedLngs: SUPPORTED_LANGUAGES as unknown as string[],
    // Collapse region variants (zh-CN, de-AT, …) to the base language, and
    // fall back to English for anything we don't ship.
    load: "languageOnly",
    nonExplicitSupportedLngs: true,
    fallbackLng: "en",
    detection: {
      // Only sniff the OS/browser locale; the persisted pref is applied
      // explicitly by App.tsx, so we don't cache in localStorage where it
      // could fight that.
      order: ["navigator"],
      caches: [],
    },
    interpolation: {
      escapeValue: false, // React already safes from xss
    },
  });

// Keep `<html lang>` in step with the UI language.
//
// It sat at the index.html default ("en") for every language we ship, so
// a Polish or Japanese interface was announced to VoiceOver as English —
// which changes pronunciation, and drives hyphenation and spellcheck off
// the wrong rules. Cheap to keep honest, and it has to react to
// `changeLanguage` too, since App.tsx applies the persisted preference
// after init.
function syncDocumentLanguage(lng: string) {
  if (typeof document !== "undefined" && lng) {
    document.documentElement.lang = lng.split("-")[0];
  }
}
syncDocumentLanguage(i18n.resolvedLanguage ?? i18n.language ?? "en");
i18n.on("languageChanged", syncDocumentLanguage);

export default i18n;
