import { useApp } from "@/lib/store";
import { Check, Type, MessageSquare, BookOpen, PenLine } from "lucide-react";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

export function AppearanceTab() {
  const { t } = useTranslation();
  const theme = useApp((s) => s.theme);
  const setTheme = useApp((s) => s.setTheme);
  const preferences = useApp((s) => s.preferences);
  const updatePref = useApp((s) => s.updatePreference);

  // Chat display/input prefs (merged in from the standalone Chat tab).
  const textSize = preferences["textSize"] || "normal";
  const sendOnEnter = preferences["sendOnEnter"] !== "false"; // default true

  const handleTheme = (id: string) => {
    setTheme(id as any);
    invoke("setting_set", { key: "theme", value: id }).catch(() => {});
  };

  const themes = [
    { id: "midnight", label: "Midnight ✧", bg: "from-[#d9ff72] to-[#0a0806]" },
    { id: "plum", label: "Plum", bg: "from-[#d9ff72] to-[#0e0816]" },
    { id: "ocean", label: "Ocean", bg: "from-[#6a7a9a] to-[#060e18]" },
    { id: "forest", label: "Forest", bg: "from-[#71d9ca] to-[#060e0a]" },
    { id: "sunset", label: "Sunset", bg: "from-[#c06014] to-[#120602]" },
    { id: "cyberpunk", label: "Cyberpunk", bg: "from-[#c07eff] to-[#060012]" },
    { id: "mocha", label: "Mocha", bg: "from-[#a08030] to-[#0e0a04]" },
    { id: "snow", label: "Snow", bg: "from-[#d9ff72] to-[#f5f0e8]" },
  ] as const;

  return (
    <div className="space-y-4">
      <p className="text-[12px] text-[var(--color-label-secondary)] mb-4">
        {t("settings.appearance.desc", "Customize the background theme of your dashboard.")}
      </p>

      <div className="grid grid-cols-2 gap-3">
        {themes.map((t) => (
          <button
            key={t.id}
            onClick={() => handleTheme(t.id)}
            className={cn(
              "relative p-3 rounded-xl border flex flex-col items-center justify-center gap-3 transition-all",
              theme === t.id
                ? "border-[var(--color-accent)] bg-[var(--color-accent-soft)]"
                : "border-[var(--color-separator)]/30 hover:border-[var(--color-label-secondary)] bg-[var(--color-fill-quaternary)]"
            )}
          >
            <div
              className={`w-12 h-12 rounded-full bg-gradient-to-br ${t.bg} shadow-inner border border-white/10`}
            />
            <span className="text-[12px] font-medium text-[var(--color-label-primary)]">
              {t.label}
            </span>
            
            {theme === t.id && (
              <div className="absolute top-2 right-2 w-5 h-5 bg-[var(--color-accent)] rounded-full flex items-center justify-center shadow-sm">
                <Check className="w-3 h-3 text-black" />
              </div>
            )}
          </button>
        ))}
      </div>

      <div className="pt-2">
        <h3 className="text-[12px] font-semibold uppercase tracking-wider text-[var(--color-label-secondary)] mb-3">
          {t("settings.drawer.chat", "Narration & Display")}
        </h3>
        <div className="space-y-2">
          <div className="surface px-4 py-3 flex items-center justify-between">
            <div className="flex items-center gap-3">
              <Type className="w-4 h-4 text-[var(--color-label-secondary)]" />
              <div>
                <div className="text-[14px] font-medium text-[var(--color-label-primary)]">{t("settings.chat.textSize.title")}</div>
                <div className="text-[12px] text-[var(--color-label-secondary)]">{t("settings.chat.textSize.desc")}</div>
              </div>
            </div>
            <select
              value={textSize}
              onChange={(e) => updatePref("textSize", e.target.value)}
              className="bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)] rounded-md px-3 py-1 text-[13px] outline-none"
            >
              <option value="small">{t("settings.chat.textSize.small")}</option>
              <option value="normal">{t("settings.chat.textSize.normal")}</option>
              <option value="large">{t("settings.chat.textSize.large")}</option>
            </select>
          </div>
        </div>
      </div>

      <div>
        <h3 className="text-[12px] font-semibold uppercase tracking-wider text-[var(--color-label-secondary)] mb-3">
          {t("settings.chat.inputBehavior.title")}
        </h3>
        <div className="space-y-2">
          <label className="surface px-4 py-3 flex items-center justify-between cursor-pointer hover:bg-[var(--color-fill-quaternary)] transition-colors">
            <div className="flex items-center gap-3">
              <MessageSquare className="w-4 h-4 text-[var(--color-label-secondary)]" />
              <div>
                <div className="text-[14px] font-medium text-[var(--color-label-primary)]">{t("settings.chat.sendOnEnter.title")}</div>
                <div className="text-[12px] text-[var(--color-label-secondary)]">{t("settings.chat.sendOnEnter.desc")}</div>
              </div>
            </div>
            <input
              type="checkbox"
              className="toggle"
              checked={sendOnEnter}
              onChange={(e) => updatePref("sendOnEnter", e.target.checked.toString())}
            />
          </label>
        </div>
      </div>

      <div>
        <h3 className="text-[12px] font-semibold uppercase tracking-wider text-[var(--color-label-secondary)] mb-3">
          {t("settings.reading.title", "Reading")}
        </h3>
        <div className="space-y-2">
          <label className="surface px-4 py-3 flex items-center justify-between cursor-pointer hover:bg-[var(--color-fill-quaternary)] transition-colors">
            <div className="flex items-center gap-3">
              <BookOpen className="w-4 h-4 text-[var(--color-label-secondary)]" />
              <div>
                <div className="text-[14px] font-medium text-[var(--color-label-primary)]">
                  {t("settings.reading.serifFont", "Serif narration font")}
                </div>
                <div className="text-[12px] text-[var(--color-label-secondary)]">
                  {t("settings.reading.serifFontDesc", "Use serif (Lora) for story text. Disable for sans-serif.")}
                </div>
              </div>
            </div>
            <input
              type="checkbox"
              className="toggle"
              checked={preferences["narrationFont"] !== "sans"}
              onChange={(e) => updatePref("narrationFont", e.target.checked ? "serif" : "sans")}
            />
          </label>
        </div>
      </div>

      <div>
        <h3 className="text-[12px] font-semibold uppercase tracking-wider text-[var(--color-label-secondary)] mb-3">
          {t("settings.narrative.title", "Narrative")}
        </h3>
        <div className="space-y-2">
          {/* Narrative style */}
          <div className="surface px-4 py-3 flex items-center justify-between">
            <div className="flex items-center gap-3">
              <PenLine className="w-4 h-4 text-[var(--color-label-secondary)]" />
              <div>
                <div className="text-[14px] font-medium text-[var(--color-label-primary)]">
                  {t("settings.narrative.style", "Narrative style")}
                </div>
                <div className="text-[12px] text-[var(--color-label-secondary)]">
                  {t("settings.narrative.styleDesc", "How the narrator writes — literary, fast, cinematic.")}
                </div>
              </div>
            </div>
            <select
              value={preferences["narrative_style"] || "default"}
              onChange={(e) => updatePref("narrative_style", e.target.value)}
              className="bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)] rounded-md px-3 py-1 text-[13px] outline-none"
            >
              <option value="default">{t("ui.default", "Default")}</option>
              <option value="literary">{t("ui.literary", "Literary")}</option>
              <option value="concise">{t("ui.concise", "Concise")}</option>
              <option value="dramatic">{t("ui.dramatic", "Dramatic")}</option>
              <option value="noir">{t("ui.noir", "Noir")}</option>
              <option value="fast">{t("ui.fast-paced", "Fast-paced")}</option>
              <option value="cinematic">{t("ui.cinematic", "Cinematic")}</option>
            </select>
          </div>
          {/* Narrative freedom */}
          <div className="surface px-4 py-3 flex items-center justify-between">
            <div className="flex items-center gap-3">
              <BookOpen className="w-4 h-4 text-[var(--color-label-secondary)]" />
              <div>
                <div className="text-[14px] font-medium text-[var(--color-label-primary)]">
                  {t("settings.narrative.freedom", "Narrative freedom")}
                </div>
                <div className="text-[12px] text-[var(--color-label-secondary)]">
                  {t("settings.narrative.freedomDesc", "How much the narrator leads vs follows.")}
                </div>
              </div>
            </div>
            <select
              value={preferences["narrative_freedom"] || "balanced"}
              onChange={(e) => updatePref("narrative_freedom", e.target.value)}
              className="bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)] rounded-md px-3 py-1 text-[13px] outline-none"
            >
              <option value="guided">{t("ui.guided", "Guided")}</option>
              <option value="balanced">{t("ui.balanced", "Balanced")}</option>
              <option value="free">{t("ui.free", "Free")}</option>
            </select>
          </div>
        </div>
      </div>
    </div>
  );
}
