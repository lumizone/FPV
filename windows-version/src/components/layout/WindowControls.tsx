import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X } from "lucide-react";
import { useTranslation } from "react-i18next";

export function WindowControls() {
  const { t } = useTranslation();
  const win = getCurrentWindow();
  return (
    <div className="relative z-[300] flex items-center h-full no-drag pointer-events-auto">
      <button
        type="button"
        onClick={() => win.minimize()}
        className="w-11 h-full flex items-center justify-center hover:bg-[var(--color-fill-quaternary)] transition-colors"
        aria-label={t("ui.minimize", "Minimize")}
      >
        <Minus className="w-3.5 h-3.5" />
      </button>
      <button
        type="button"
        onClick={() => win.toggleMaximize()}
        className="w-11 h-full flex items-center justify-center hover:bg-[var(--color-fill-quaternary)] transition-colors"
        aria-label={t("ui.maximize", "Maximize")}
      >
        <Square className="w-3 h-3" />
      </button>
      <button
        type="button"
        onClick={() => win.close()}
        className="w-11 h-full flex items-center justify-center hover:bg-red-600 hover:text-white transition-colors"
        aria-label={t("ui.close", "Close")}
      >
        <X className="w-3.5 h-3.5" />
      </button>
    </div>
  );
}
