import { useEffect, useState } from "react";
import { Activity, HardDrive, Globe, RefreshCw, Trash2, Download, Upload } from "lucide-react";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { settingGet, settingSet } from "@/lib/tauri";
import { useApp } from "@/lib/store";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

interface GenerationMetric {
  timestamp: number;
  stage: string;
  provider: string;
  model: string;
  input_tokens_estimate: number;
  output_tokens_estimate: number;
  duration_ms: number;
  streaming: boolean;
}

export function AboutTab() {
  const { t } = useTranslation();
  const [version, setVersion] = useState("");
  const [metrics, setMetrics] = useState<GenerationMetric[]>([]);
  const [cloudLimit, setCloudLimit] = useState("0");
  const [projectTransfer, setProjectTransfer] = useState<"idle" | "exporting" | "importing">("idle");
  const refreshWorlds = useApp((state) => state.refreshWorlds);
  const clearRuntimeState = useApp((state) => state.clearRuntimeState);

  const loadMetrics = () => {
    invoke<GenerationMetric[]>("metrics_read").then(setMetrics).catch(() => setMetrics([]));
  };

  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
    loadMetrics();
    settingGet("cloud_daily_limit_usd").then((value) => setCloudLimit(value || "0")).catch(() => {});
  }, []);

  const cloudTurns = metrics.filter((metric) => metric.provider !== "ollama").length;
  const inputTokens = metrics.reduce((sum, metric) => sum + metric.input_tokens_estimate, 0);
  const outputTokens = metrics.reduce((sum, metric) => sum + metric.output_tokens_estimate, 0);
  const totalDuration = metrics.reduce((sum, metric) => sum + metric.duration_ms, 0);
  const referenceCost = (inputTokens / 1_000_000) * 3 + (outputTokens / 1_000_000) * 15;
  const resetMetrics = async () => {
    await invoke("metrics_reset").catch(() => {});
    setMetrics([]);
  };
  const updateCloudLimit = (value: string) => {
    if (!/^\d*(\.\d{0,2})?$/.test(value)) return;
    setCloudLimit(value);
    settingSet("cloud_daily_limit_usd", value || "0").catch(() => {});
  };

  const exportProject = async () => {
    const destination = await save({
      defaultPath: "FPV-project.fpv-project",
      filters: [{ name: "FPV Project", extensions: ["fpv-project"] }],
    });
    if (!destination || Array.isArray(destination)) return;
    setProjectTransfer("exporting");
    try {
      const summary = await invoke<{ worlds: number; sessions: number; assets: number }>("project_export", { dest_path: destination });
      toast.success(t("ui.project-backup-created", "Project backup created"), { description: t("ui.backup-summary", "{{worlds}} worlds, {{sessions}} adventures, {{assets}} assets.", summary) });
    } catch (error) {
      toast.error(t("ui.project-backup-failed", "Project backup failed"), { description: String(error) });
    } finally {
      setProjectTransfer("idle");
    }
  };

  const importProject = async () => {
    const source = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "FPV Project", extensions: ["fpv-project"] }],
    });
    if (!source || Array.isArray(source)) return;
    if (!window.confirm(t("ui.restore-project-confirm", "Restore this project? Current worlds, adventures, and story data will be replaced."))) return;
    setProjectTransfer("importing");
    try {
      const summary = await invoke<{ worlds: number; sessions: number; assets: number }>("project_import", { source_path: source });
      clearRuntimeState();
      await refreshWorlds();
      toast.success(t("ui.project-restored", "Project restored"), { description: t("ui.backup-summary", "{{worlds}} worlds, {{sessions}} adventures, {{assets}} assets.", summary) });
    } catch (error) {
      toast.error(t("ui.project-restore-failed", "Project restore failed"), { description: String(error) });
    } finally {
      setProjectTransfer("idle");
    }
  };

  const resetAllData = async () => {
    if (!window.confirm(t("ui.reset-all-data-confirm-1", "Delete all FPV worlds, stories, images, models, logs and saved provider keys? This cannot be undone."))) return;
    if (!window.confirm(t("ui.reset-all-data-confirm-2", "Final confirmation: permanently erase all local FPV data?"))) return;
    try {
      await invoke("reset_all_data");
      clearRuntimeState();
      await refreshWorlds();
      toast.success(t("ui.all-local-data-deleted", "All local FPV data was deleted"));
    } catch (error) {
      toast.error(t("ui.reset-failed", "Reset failed"), { description: String(error) });
    }
  };

  return (
    <div className="space-y-6">
      <div className="text-center py-4">
        <h2 className="text-xl font-display tracking-[0.04em] text-[var(--color-label-primary)]">FPV</h2>
        <p className="text-xs text-[var(--color-label-tertiary)] mt-1">{t("ui.first-person-viewpoint", "First Person Viewpoint")}</p>
        <p className="text-xs text-[var(--color-label-tertiary)] mt-3">
          v{version} · Direct
        </p>
        <p className="text-xs text-[var(--color-label-tertiary)] mt-1">macOS</p>
      </div>

      <div className="p-4 rounded-lg bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)]">
        <p className="text-xs text-[var(--color-label-secondary)] leading-relaxed">
          {t("ui.fpv-is-an-ai-powered-interactive-narrative-app-c", "FPV is an AI-powered interactive narrative app. Create immersive\n          stories in any genre — fantasy, sci-fi, horror, romance, and more —\n           running locally on your computer by default. Optional BYOK cloud\n           providers are only used when you explicitly connect and select one.")}
        </p>
      </div>

      <div className="p-4 rounded-lg bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)] space-y-3">
        <div>
          <h3 className="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--color-label-tertiary)]">{t("ui.project-backup", "Project backup")}</h3>
          <p className="mt-1 text-[11px] leading-relaxed text-[var(--color-label-secondary)]">
            {t("ui.save-worlds-adventures-codex-story-state-paths-a", "Save worlds, adventures, Codex, Story State, paths, and generated images. API keys, model weights, and provider settings are never included.")}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            onClick={exportProject}
            disabled={projectTransfer !== "idle"}
            className="inline-flex items-center gap-2 rounded-lg bg-[var(--color-accent)] px-3 py-2 text-[11px] font-semibold text-black transition-colors hover:brightness-110 disabled:opacity-50"
          >
            <Download className="h-3.5 w-3.5" />
            {projectTransfer === "exporting" ? "Creating backup..." : "Export project"}
          </button>
          <button
            type="button"
            onClick={importProject}
            disabled={projectTransfer !== "idle"}
            className="inline-flex items-center gap-2 rounded-lg border border-[var(--color-separator)] px-3 py-2 text-[11px] font-semibold text-[var(--color-label-primary)] transition-colors hover:border-[var(--color-accent)] disabled:opacity-50"
          >
            <Upload className="h-3.5 w-3.5" />
            {projectTransfer === "importing" ? "Restoring project..." : "Restore project"}
          </button>
        </div>
      </div>

      <div className="p-4 rounded-lg border border-red-500/25 bg-red-500/5 space-y-2">
        <h3 className="text-[11px] font-semibold uppercase tracking-[0.1em] text-red-300">{t("ui.delete-all-local-data", "Delete all local data")}</h3>
        <p className="text-[11px] leading-relaxed text-[var(--color-label-secondary)]">
          {t("ui.removes-worlds-stories-generated-assets-download", "Removes worlds, stories, generated assets, downloaded local models, logs and saved API keys.")}
        </p>
        <button type="button" onClick={() => void resetAllData()} className="inline-flex items-center gap-2 rounded-lg border border-red-400/40 px-3 py-2 text-[11px] font-semibold text-red-300 hover:bg-red-500/10">
          <Trash2 className="h-3.5 w-3.5" /> {t("ui.delete-all-local-data", "Delete all local data")}
        </button>
      </div>

      <div className="p-4 rounded-lg bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)] space-y-2">
        <h3 className="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--color-label-tertiary)]">{t("ui.keyboard-shortcuts", "Keyboard Shortcuts")}</h3>
        <div className="grid grid-cols-2 gap-1 text-xs">
          <span className="text-[var(--color-label-tertiary)]">{t("ui.settings", "Settings")}</span>
          <span className="text-[var(--color-label-primary)] text-right font-mono">⌘,</span>
          <span className="text-[var(--color-label-tertiary)]">{t("ui.new-world", "New world")}</span>
          <span className="text-[var(--color-label-primary)] text-right font-mono">⌘⇧N</span>
          <span className="text-[var(--color-label-tertiary)]">{t("ui.home", "Home")}</span>
          <span className="text-[var(--color-label-primary)] text-right font-mono">⌘⇧K</span>
        </div>
      </div>

      <div className="p-4 rounded-lg bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)] space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Activity className="w-4 h-4 text-[var(--color-accent)]" />
            <h3 className="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--color-label-tertiary)]">{t("ui.generation-diagnostics", "Generation Diagnostics")}</h3>
          </div>
          <div className="flex items-center gap-1">
            <button onClick={loadMetrics} className="p-1.5 rounded-md text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)]" title={t("ui.refresh-metrics", "Refresh metrics")} aria-label={t("ui.refresh-metrics", "Refresh metrics")}>
              <RefreshCw className="w-3.5 h-3.5" />
            </button>
            <button onClick={resetMetrics} className="p-1.5 rounded-md text-[var(--color-label-tertiary)] hover:text-red-400" title={t("ui.clear-metrics", "Clear metrics")} aria-label={t("ui.clear-metrics", "Clear metrics")}>
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>
        <div className="grid grid-cols-2 gap-2 text-xs">
          <div><span className="text-[var(--color-label-tertiary)]">{t("ui.generations", "Generations")}</span><strong className="block text-[var(--color-label-primary)]">{metrics.length}</strong></div>
          <div><span className="text-[var(--color-label-tertiary)]">{t("ui.cloud-turns", "Cloud turns")}</span><strong className="block text-[var(--color-label-primary)]">{cloudTurns}</strong></div>
          <div><span className="text-[var(--color-label-tertiary)]">{t("ui.input-tokens-est", "Input tokens est.")}</span><strong className="block text-[var(--color-label-primary)]">{inputTokens.toLocaleString()}</strong></div>
          <div><span className="text-[var(--color-label-tertiary)]">{t("ui.output-tokens-est", "Output tokens est.")}</span><strong className="block text-[var(--color-label-primary)]">{outputTokens.toLocaleString()}</strong></div>
          <div><span className="text-[var(--color-label-tertiary)]">{t("ui.total-inference-time", "Total inference time")}</span><strong className="block text-[var(--color-label-primary)]">{(totalDuration / 1000).toFixed(1)}s</strong></div>
          <div><span className="text-[var(--color-label-tertiary)]">{t("ui.reference-cloud-cost", "Reference cloud cost")}</span><strong className="block text-[var(--color-label-primary)]">${referenceCost.toFixed(4)}</strong></div>
        </div>
        <p className="text-[10px] leading-relaxed text-[var(--color-label-quaternary)]">{t("ui.cost-estimate-uses-reference-rates-of-3-m-input", "Cost estimate uses reference rates of $3/M input and $15/M output. It is local-only and not an invoice.")}</p>
        <label className="flex items-center justify-between gap-3 text-[11px] text-[var(--color-label-secondary)]">
          {t("ui.daily-cloud-limit-usd", "Daily cloud limit (USD)")}
          <input value={cloudLimit} onChange={(event) => updateCloudLimit(event.target.value)} inputMode="decimal" className="w-20 rounded-md bg-[var(--color-fill-tertiary)] px-2 py-1 text-right text-[11px] text-[var(--color-label-primary)] outline-none border border-[var(--color-separator)]" aria-label={t("ui.daily-cloud-limit-in-usd", "Daily cloud limit in USD")} />
        </label>
      </div>

      <div className="space-y-2">
        <button
          onClick={() => openUrl("https://firstpersonviewpoint.com")}
          className="w-full flex items-center justify-between p-3 rounded-lg hover:bg-[var(--color-fill-quaternary)] transition-colors"
        >
          <div className="flex items-center gap-3">
            <Globe className="w-4 h-4 text-[var(--color-label-tertiary)]" />
            <span className="text-sm text-[var(--color-label-secondary)]">{t("ui.website", "Website")}</span>
          </div>
          <span className="text-xs text-[var(--color-label-tertiary)]">firstpersonviewpoint.com</span>
        </button>

        <button
          onClick={() => openUrl("https://github.com/lumizone/fpv-desktop")}
          className="w-full flex items-center justify-between p-3 rounded-lg hover:bg-[var(--color-fill-quaternary)] transition-colors"
        >
          <div className="flex items-center gap-3">
            <HardDrive className="w-4 h-4 text-[var(--color-label-tertiary)]" />
            <span className="text-sm text-[var(--color-label-secondary)]">{t("ui.github", "GitHub")}</span>
          </div>
          <span className="text-xs text-[var(--color-label-tertiary)]">github.com/lumizone/fpv-desktop</span>
        </button>
      </div>

      <div className="p-4 rounded-lg bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)]">
        <p className="text-xs text-[var(--color-label-secondary)] leading-relaxed">
          Built with Tauri 2, React, Rust, Ollama, and stable-diffusion.cpp.
          All your stories, worlds, and settings are stored locally at{" "}
          <code className="bg-[var(--color-fill-quaternary)] px-1 py-0.5 rounded text-[11px] font-mono border border-[var(--color-separator)]">
            ~/Library/Application Support/com.lumizone.fpvdesktop/
          </code>
          . You can back this up at any time.
        </p>
      </div>

      <div className="p-4 rounded-lg bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)]">
        <p className="text-xs text-[var(--color-label-secondary)] leading-relaxed">
          {t("ui.fpv-is-built-on-open-source-work-local-models-ru", "FPV is built on open-source work. Local models run via Ollama and\n          stable-diffusion.cpp. Full license terms ship with each model.")}
        </p>
      </div>
    </div>
  );
}
