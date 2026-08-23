import { useEffect, useState } from "react";
import { AppWindow, BatteryCharging, Brain, Cable, Globe, PanelTopClose, Power } from "lucide-react";
import { useApp } from "@/lib/store";
import { useTranslation } from "react-i18next";
import { enable as autostartEnable, disable as autostartDisable, isEnabled as autostartIsEnabled } from "@tauri-apps/plugin-autostart";

export function GeneralTab() {
  const preferences = useApp((s) => s.preferences);
  const updatePref = useApp((s) => s.updatePreference);
  const { t, i18n } = useTranslation();
  const [startAtLogin, setStartAtLogin] = useState(false);

  useEffect(() => {
    autostartIsEnabled().then(setStartAtLogin).catch(() => {});
  }, []);

  const toggleAutostart = async () => {
    try {
      if (startAtLogin) {
        await autostartDisable();
      } else {
        await autostartEnable();
      }
      setStartAtLogin(!startAtLogin);
    } catch (e: any) {
      console.error("autostart toggle failed", e);
    }
  };

  const togglePref = (key: string) => {
    const current = preferences[key] === "true";
    updatePref(key, current ? "false" : "true");
  };


  return (
    <div className="space-y-6">
      {/* Language */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Globe className="w-4 h-4 text-[var(--color-label-tertiary)]" />
          <div>
            <div className="text-sm text-[var(--color-label-primary)]">{t("settings.general.language.title", "Language")}</div>
            <div className="text-xs text-[var(--color-label-tertiary)]">{i18n.language}</div>
          </div>
        </div>
        <select
          value={i18n.language}
          onChange={(e) => {
            i18n.changeLanguage(e.target.value);
            updatePref("language", e.target.value);
          }}
          className="bg-[var(--color-fill-quaternary)] text-[var(--color-label-primary)] text-sm rounded-lg px-2 py-1 border border-[var(--color-separator)]"
        >
          {["en", "pl", "zh", "ko", "ja", "de", "es"].map((l) => (
            <option key={l} value={l}>{l.toUpperCase()}</option>
          ))}
        </select>
      </div>

      {/* Start at login */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Power className="w-4 h-4 text-[var(--color-label-tertiary)]" />
          <div>
            <div className="text-sm text-[var(--color-label-primary)]">{t("settings.general.autostart", "Start at Login")}</div>
          </div>
        </div>
        <button
          onClick={toggleAutostart}
          type="button"
          role="switch"
          aria-checked={startAtLogin}
          aria-label={t("settings.general.autostart", "Start at Login")}
          className={`w-10 h-5 rounded-full transition-colors ${startAtLogin ? "bg-[var(--color-accent)]" : "bg-[var(--color-fill-primary)]"}`}
        >
          <div className={`w-4 h-4 rounded-full bg-[var(--color-label-primary)] transition-transform ${startAtLogin ? "translate-x-[22px]" : "translate-x-0.5"}`} />
        </button>
      </div>

      {/* Blur on background */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <AppWindow className="w-4 h-4 text-[var(--color-label-tertiary)]" />
          <div>
            <div className="text-sm text-[var(--color-label-primary)]">{t("settings.general.blur", "Blur on Background")}</div>
          </div>
        </div>
        <button
          onClick={() => togglePref("blurOnBackground")}
          type="button"
          role="switch"
          aria-checked={preferences["blurOnBackground"] === "true"}
          aria-label={t("settings.general.blur", "Blur on Background")}
          className={`w-10 h-5 rounded-full transition-colors ${preferences["blurOnBackground"] === "true" ? "bg-[var(--color-accent)]" : "bg-[var(--color-fill-primary)]"}`}
        >
          <div className={`w-4 h-4 rounded-full bg-[var(--color-label-primary)] transition-transform ${preferences["blurOnBackground"] === "true" ? "translate-x-[22px]" : "translate-x-0.5"}`} />
        </button>
      </div>

      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <PanelTopClose className="w-4 h-4 text-[var(--color-label-tertiary)]" />
          <div>
            <div className="text-sm text-[var(--color-label-primary)]">{t("settings.general.tray.title", "Hide to Tray")}</div>
            <div className="text-xs text-[var(--color-label-tertiary)]">{t("settings.general.tray.desc", "Keep app running in menu bar when closed")}</div>
          </div>
        </div>
        <button
          onClick={() => togglePref("hideToTray")}
          type="button"
          role="switch"
          aria-checked={preferences["hideToTray"] === "true"}
          aria-label={t("settings.general.tray.title", "Hide to Tray")}
          className={`w-10 h-5 rounded-full transition-colors ${preferences["hideToTray"] === "true" ? "bg-[var(--color-accent)]" : "bg-[var(--color-fill-primary)]"}`}
        >
          <div className={`w-4 h-4 rounded-full bg-[var(--color-label-primary)] transition-transform ${preferences["hideToTray"] === "true" ? "translate-x-[22px]" : "translate-x-0.5"}`} />
        </button>
      </div>

      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <BatteryCharging className="w-4 h-4 text-[var(--color-label-tertiary)]" />
          <div>
            <div className="text-sm text-[var(--color-label-primary)]">{t("settings.general.low_power.title", "Low Power Mode")}</div>
            <div className="text-xs text-[var(--color-label-tertiary)]">{t("settings.general.low_power.desc", "Unload local models quickly and reduce memory use")}</div>
          </div>
        </div>
        <button
          onClick={() => togglePref("lowPowerMode")}
          type="button"
          role="switch"
          aria-checked={preferences["lowPowerMode"] === "true"}
          aria-label={t("settings.general.low_power.title", "Low Power Mode")}
          className={`w-10 h-5 rounded-full transition-colors ${preferences["lowPowerMode"] === "true" ? "bg-[var(--color-accent)]" : "bg-[var(--color-fill-primary)]"}`}
        >
          <div className={`w-4 h-4 rounded-full bg-[var(--color-label-primary)] transition-transform ${preferences["lowPowerMode"] === "true" ? "translate-x-[22px]" : "translate-x-0.5"}`} />
        </button>
      </div>

      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Cable className="w-4 h-4 text-[var(--color-label-tertiary)]" />
          <div>
            <div className="text-sm text-[var(--color-label-primary)]">{t("settings.general.images_ac.title", "Automatic Images on AC Only")}</div>
            <div className="text-xs text-[var(--color-label-tertiary)]">{t("settings.general.images_ac.desc", "Pause automatic image generation while using the battery")}</div>
          </div>
        </div>
        <button
          onClick={() => updatePref("autoImagesAcOnly", preferences["autoImagesAcOnly"] === "false" ? "true" : "false")}
          type="button"
          role="switch"
          aria-checked={preferences["autoImagesAcOnly"] !== "false"}
          aria-label={t("settings.general.images_ac.title", "Automatic Images on AC Only")}
          className={`w-10 h-5 rounded-full transition-colors ${preferences["autoImagesAcOnly"] !== "false" ? "bg-[var(--color-accent)]" : "bg-[var(--color-fill-primary)]"}`}
        >
          <div className={`w-4 h-4 rounded-full bg-[var(--color-label-primary)] transition-transform ${preferences["autoImagesAcOnly"] !== "false" ? "translate-x-[22px]" : "translate-x-0.5"}`} />
        </button>
      </div>

      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Brain className="w-4 h-4 text-[var(--color-label-tertiary)]" />
          <div>
            <div className="text-sm text-[var(--color-label-primary)]">{t("settings.general.semantic_memory.title", "Semantic Memory")}</div>
            <div className="text-xs text-[var(--color-label-tertiary)]">{t("settings.general.semantic_memory.desc", "Use an additional local model for long-term story recall")}</div>
          </div>
        </div>
        <button
          onClick={() => togglePref("semanticMemoryEnabled")}
          type="button"
          role="switch"
          aria-checked={preferences["semanticMemoryEnabled"] === "true"}
          aria-label={t("settings.general.semantic_memory.title", "Semantic Memory")}
          className={`w-10 h-5 rounded-full transition-colors ${preferences["semanticMemoryEnabled"] === "true" ? "bg-[var(--color-accent)]" : "bg-[var(--color-fill-primary)]"}`}
        >
          <div className={`w-4 h-4 rounded-full bg-[var(--color-label-primary)] transition-transform ${preferences["semanticMemoryEnabled"] === "true" ? "translate-x-[22px]" : "translate-x-0.5"}`} />
        </button>
      </div>

    </div>
  );
}
