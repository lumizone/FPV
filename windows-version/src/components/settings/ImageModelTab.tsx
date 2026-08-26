import { useEffect, useState } from "react";
import { Download, RefreshCw, Loader2, Image, XCircle, HardDrive, Cloud, CheckCircle2, CircleAlert, Cpu } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { useApp } from "@/lib/store";
import { isWindows } from "@/lib/platform";
import {
  imageLocalModels,
  imageLocalModelGet,
  imageLocalModelSet,
  imageLocalModelDelete,
  imageLocalPrewarm,
  imageDownloadCancel,
  imageGenerate,
  imageLocalCheck,
  imageProviderGet,
  imageProviderSet,
  imageCloudModels,
  imageCloudModelGet,
  imageCloudModelSet,
  byokList,
  gpuRuntimeStatus,
  gpuRuntimeRetryImage,
  type LocalModelChoice,
  type GpuRuntimeStatus,
  type ImageProviderId,
} from "@/lib/tauri";
import { CLOUD_IMAGE_PROVIDERS, CLOUD_PROVIDER_READY_KEY, CLOUD_PROVIDER_READY_FALLBACK } from "@/lib/imageProviders";

/// Settings tab for the image-generation model (stable-diffusion.cpp + cloud).
/// Lists the sd.cpp model catalog, downloads weights, sets the active
/// image model used for scene and cover generation.
export function ImageModelTab() {
  const { t } = useTranslation();
  const [models, setModels] = useState<LocalModelChoice[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [activeModel, setActiveModel] = useState<string | null>(null);
  const [localEngineReady, setLocalEngineReady] = useState<boolean | null>(null);
  const [byokProviders, setByokProviders] = useState<string[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [downloadPct, setDownloadPct] = useState<number | null>(null);
  const [testState, setTestState] = useState<"idle" | "running" | "ok" | "fail">("idle");
  const [gpuStatus, setGpuStatus] = useState<GpuRuntimeStatus | null>(null);
  const [gpuRetrying, setGpuRetrying] = useState(false);
  const preferences = useApp((s) => s.preferences);
  const updatePreference = useApp((s) => s.updatePreference);
  const imageFrequency = preferences["story_image_frequency"] || "off";
  const imageStyle = preferences["story_image_style"] || "cinematic";
  const imageQuality = preferences["story_image_quality"] || "balanced";
  const imageMaxAuto = preferences["story_image_max_auto"] || "10";
  const [selectedProvider, setSelectedProvider] = useState<ImageProviderId>("local");
  // Live cloud image models, fetched from each provider's own API (Settings →
  // AI Cloud Connection key required), plus the user's saved model choice.
  const [cloudModels, setCloudModels] = useState<Record<string, string[]>>({});
  const [cloudModelsLoading, setCloudModelsLoading] = useState<Record<string, boolean>>({});
  const [cloudModelsError, setCloudModelsError] = useState<Record<string, string | null>>({});
  const [cloudModel, setCloudModel] = useState<Record<string, string>>({});
  const [cloudModelSaving, setCloudModelSaving] = useState(false);
  const [manualCloudModel, setManualCloudModel] = useState<Record<string, string>>({});

  // Whether each cloud image provider's BYOK key is present, resolved
  // through the shared registry (src/lib/imageProviders.ts).
  const keySaved = (p: ImageProviderId) =>
    byokProviders.includes(CLOUD_IMAGE_PROVIDERS.find((e) => e.id === p)?.byokKey ?? "");

  const refresh = () => {
    setLoading(true);
    setLoadError(null);
    Promise.all([imageLocalModels(), imageLocalModelGet(), imageLocalCheck(), byokList(), imageProviderGet()])
      .then(([m, active, localCheck, byok, provider]) => {
        setModels(m);
        setActiveModel(active);
        setLocalEngineReady(localCheck.local_ready);
        setByokProviders(byok.providers);
        const keyProvider = CLOUD_IMAGE_PROVIDERS.find((e) => e.id === provider)?.byokKey;
        setSelectedProvider(
          provider === "local" || (keyProvider && byok.providers.includes(keyProvider))
            ? (provider as ImageProviderId)
            : "local"
        );
      })
      .catch((e) => setLoadError((e as { message?: string })?.message ?? t("settings.image.load_error", "Could not load image models.")))
      .finally(() => setLoading(false));
  };

  // Listen to sd.cpp weight-download progress.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<{ phase: string; percent: number | null; message: string | null }>(
          "image:prewarm",
          (e) => {
            setDownloadPct(e.payload.percent);
            if (e.payload.phase === "done") {
              setBusy(null);
              setDownloadPct(null);
              refresh();
            }
          }
        )
      )
      .then((u) => { unlisten = u; })
      .catch(() => {});
    return () => { unlisten?.(); };
  }, []);

  useEffect(() => { refresh(); }, []);

  // Windows-only: the local sd.cpp backend's own GPU build, separate from
  // model WEIGHTS above. `gpuRuntimeStatus()` returns
  // platform_supported: false on macOS rather than erroring, so this is
  // safe to call unconditionally.
  const refreshGpu = () => {
    gpuRuntimeStatus().then(setGpuStatus).catch(() => {});
  };
  useEffect(() => { refreshGpu(); }, []);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<string>("gpu-setup", (e) => {
          if (e.payload === "ready" || e.payload === "error") refreshGpu();
        })
      )
      .then((u) => { unlisten = u; })
      .catch(() => {});
    return () => { unlisten?.(); };
  }, []);

  const handleGpuRetry = async () => {
    if (gpuRetrying) return;
    setGpuRetrying(true);
    try {
      await gpuRuntimeRetryImage();
      refreshGpu();
    } catch (e: any) {
      toast.error(
        (e as { message?: string })?.message ??
          t("settings.image.gpu_retry_failed", "GPU setup retry failed — continuing on CPU.")
      );
    } finally {
      setGpuRetrying(false);
    }
  };

  // Download weights for a model: set it active, then prewarm (downloads
  // the GGUF weights via sd.cpp). Progress arrives over the `image:prewarm`
  // event; when it emits "done" the model is ready.
  const handleDownload = async (id: string) => {
    try {
      await imageLocalModelSet(id);
      setActiveModel(id);
      setBusy(id);
      setDownloadPct(0);
      await imageLocalPrewarm();
      refresh();
    } catch (e: any) {
      console.error("image model download failed", e);
      setBusy(null);
      setDownloadPct(null);
      toast.error((e as { message?: string })?.message ?? t("settings.image.download_failed", "Image model download failed."));
    }
  };

  const handleCancelDownload = async () => {
    await imageDownloadCancel().catch(() => {});
  };

  const handleSetDefault = async (id: string) => {
    try {
      await imageLocalModelSet(id);
      setActiveModel(id);
    } catch (e: any) {
      console.error("set image model failed", e);
    }
  };

  const handleDelete = async (id: string) => {
    if (!window.confirm(t("settings.image.delete_confirm", "Delete image model \"{{id}}\" weights? This cannot be undone.", { id }))) return;
    setBusy(id);
    try {
      await imageLocalModelDelete(id);
      refresh();
    } catch (e: any) {
      console.error("delete image model failed", e);
    } finally {
      setBusy(null);
    }
  };

  const handleTest = async () => {
    setTestState("running");
    try {
      await imageGenerate({ prompt: "test", style: "anime" });
      setTestState("ok");
    } catch (e: any) {
      setTestState("fail");
      toast.error(
          (e as { message?: string })?.message ??
          t("settings.image.test_failed", "Image generation failed - install a model first.")
      );
    }
  };

  const readyModels = models.filter((m) => m.ready);
  const localProviderReady = localEngineReady === true && readyModels.length > 0;

  const handleProviderChange = async (provider: ImageProviderId) => {
    if (provider !== "local" && !keySaved(provider)) return;
    try {
      await imageProviderSet(provider);
      setSelectedProvider(provider);
    } catch (error) {
      toast.error((error as { message?: string })?.message ?? t("settings.image.provider_error", "Could not change the image provider."));
    }
  };

  // Live model discovery for the selected cloud provider — fetched from the
  // provider's own API, never a hand-maintained list, so a newly released
  // model needs no app update (same approach as the chat Narrator tab).
  const fetchCloudModels = async (providerId: ImageProviderId) => {
    setCloudModelsLoading((s) => ({ ...s, [providerId]: true }));
    setCloudModelsError((s) => ({ ...s, [providerId]: null }));
    try {
      const ids = await imageCloudModels(providerId);
      setCloudModels((s) => ({ ...s, [providerId]: ids }));
    } catch (e: any) {
      setCloudModelsError((s) => ({
        ...s,
        [providerId]: (e as { message?: string })?.message ?? t("settings.image.load_error", "Could not load image models."),
      }));
    } finally {
      setCloudModelsLoading((s) => ({ ...s, [providerId]: false }));
    }
  };

  const loadCloudModel = async (providerId: ImageProviderId) => {
    try {
      const m = await imageCloudModelGet(providerId);
      setCloudModel((s) => ({ ...s, [providerId]: m }));
    } catch { /* default model is applied server-side */ }
  };

  const selectCloudModel = async (providerId: ImageProviderId, model: string) => {
    setCloudModelSaving(true);
    try {
      await imageCloudModelSet(providerId, model);
      setCloudModel((s) => ({ ...s, [providerId]: model }));
    } catch (e: any) {
      toast.error((e as { message?: string })?.message ?? t("settings.image.provider_error", "Could not change the image model."));
    } finally {
      setCloudModelSaving(false);
    }
  };

  const selectedCloud = selectedProvider !== "local" ? selectedProvider : null;

  useEffect(() => {
    if (selectedCloud && keySaved(selectedCloud)) {
      fetchCloudModels(selectedCloud);
      loadCloudModel(selectedCloud);
    }
    // Re-fetch only when the SELECTED provider changes, not on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedCloud]);

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-sm font-semibold text-[var(--color-label-primary)] mb-1">
          {t("settings.image.title", "Image Generation")}
        </h3>
        <p className="text-[11px] text-[var(--color-label-tertiary)] mb-4">
          {t("settings.image.desc", "Local image generation via stable-diffusion.cpp. Generates scene illustrations and story covers entirely on your machine.")}
        </p>

        <fieldset className="mb-4" aria-describedby="image-provider-description">
          <legend className="text-[12px] font-semibold uppercase tracking-wider text-[var(--color-label-secondary)]">
            {t("settings.image.provider", "Image provider")}
          </legend>
          <p id="image-provider-description" className="mt-1 text-[11px] text-[var(--color-label-tertiary)]">
            {t("settings.image.provider_desc", "Choose local generation or your connected OpenAI Image API.")}
          </p>
          <div className="mt-3 grid gap-2 sm:grid-cols-2">
            <label className={`surface flex min-h-24 cursor-pointer items-start gap-3 px-3 py-3 transition-colors ${selectedProvider === "local" ? "border-[var(--color-accent)]/60 bg-[var(--color-accent-soft)]" : "hover:border-[var(--color-label-tertiary)]/40"}`}>
              <input type="radio" name="image-provider" value="local" checked={selectedProvider === "local"} onChange={() => handleProviderChange("local")} className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--color-accent)]" />
              <HardDrive aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0 text-[var(--color-accent)]" />
              <span className="min-w-0">
                <span className="block text-[13px] font-medium text-[var(--color-label-primary)]">{t("settings.image.provider_local", "Local sd.cpp")}</span>
                <span className="mt-0.5 block text-[11px] leading-4 text-[var(--color-label-tertiary)]">{t("settings.image.provider_local_desc", "Private generation on this Mac.")}</span>
                <span className={`mt-2 flex items-center gap-1 text-[11px] ${localProviderReady ? "text-[var(--color-system-green)]" : "text-[var(--color-warm)]"}`}>
                  {localProviderReady ? <CheckCircle2 aria-hidden="true" className="h-3.5 w-3.5" /> : <CircleAlert aria-hidden="true" className="h-3.5 w-3.5" />}
                  {localProviderReady ? t("settings.image.provider_local_ready", "Ready") : t("settings.image.provider_local_setup", "Download a model to get started")}
                </span>
              </span>
            </label>

            {CLOUD_IMAGE_PROVIDERS.map((entry) => {
              const saved = byokProviders.includes(entry.byokKey);
              const selected = selectedProvider === entry.id;
              return (
                <label key={entry.id} className={`surface flex min-h-24 items-start gap-3 px-3 py-3 transition-colors ${saved ? "cursor-pointer hover:border-[var(--color-label-tertiary)]/40" : "cursor-not-allowed opacity-60"} ${selected ? "border-[var(--color-accent)]/60 bg-[var(--color-accent-soft)]" : ""}`}>
                  <input type="radio" name="image-provider" value={entry.id} checked={selected} onChange={() => handleProviderChange(entry.id)} disabled={!saved} className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--color-accent)] disabled:cursor-not-allowed" />
                  <Cloud aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0 text-[var(--color-accent)]" />
                  <span className="min-w-0">
                    <span className="block text-[13px] font-medium text-[var(--color-label-primary)]">{t(entry.labelKey, entry.labelFallback)}</span>
                    <span className="mt-0.5 block text-[11px] leading-4 text-[var(--color-label-tertiary)]">{t(entry.descKey, entry.descFallback)}</span>
                    <span className={`mt-2 flex items-center gap-1 text-[11px] ${saved ? "text-[var(--color-system-green)]" : "text-[var(--color-label-tertiary)]"}`}>
                      {saved ? <CheckCircle2 aria-hidden="true" className="h-3.5 w-3.5" /> : <CircleAlert aria-hidden="true" className="h-3.5 w-3.5" />}
                      {saved ? t(CLOUD_PROVIDER_READY_KEY, CLOUD_PROVIDER_READY_FALLBACK) : t(entry.setupKey, entry.setupFallback)}
                    </span>
                  </span>
                </label>
              );
            })}
          </div>
        </fieldset>

        {/* Cloud model — picked live from the provider's API */}
        {selectedCloud && keySaved(selectedCloud) && (
          <div className="mb-4 p-3 rounded-lg bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)]">
            <div className="flex items-center justify-between mb-1">
              <span className="text-[11px] uppercase tracking-wider text-[var(--color-label-secondary)] font-semibold">
                Cloud model
              </span>
              <button
                onClick={() => fetchCloudModels(selectedCloud)}
                disabled={cloudModelsLoading[selectedCloud]}
                aria-label="Refresh cloud image models"
                className="text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] transition-colors disabled:opacity-40"
              >
                <RefreshCw className={cloudModelsLoading[selectedCloud] ? "w-3.5 h-3.5 animate-spin" : "w-3.5 h-3.5"} />
              </button>
            </div>
            <p className="text-[11px] text-[var(--color-label-tertiary)] mb-2">
              Fetched live from the provider's API — new models appear without an app update.
            </p>
            {cloudModelsLoading[selectedCloud] ? (
              <div className="flex items-center gap-2 text-[var(--color-label-tertiary)] text-[12px] py-1">
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
                {t("ui.loading", "Loading...")}
              </div>
            ) : cloudModelsError[selectedCloud] ? (
              <div role="alert" className="flex items-center justify-between gap-2 rounded-lg border border-red-500/20 bg-red-500/10 px-3 py-2 text-[12px] text-red-300 mb-2">
                <span className="truncate">{cloudModelsError[selectedCloud]}</span>
                <button onClick={() => fetchCloudModels(selectedCloud)} className="shrink-0 underline hover:no-underline">
                  {t("settings.models.retry", "Retry")}
                </button>
              </div>
            ) : (
              <div className="flex flex-wrap gap-1.5 mb-2">
                {(cloudModels[selectedCloud] ?? []).map((id) => {
                  const active = cloudModel[selectedCloud] === id;
                  return (
                    <button
                      key={id}
                      onClick={() => selectCloudModel(selectedCloud, id)}
                      disabled={cloudModelSaving}
                      className={
                        active
                          ? "px-3 py-1.5 rounded-lg text-[12px] font-medium border border-[var(--color-accent)] bg-[var(--color-accent-soft)] text-[var(--color-accent)] transition-colors"
                          : "px-3 py-1.5 rounded-lg text-[12px] font-medium border border-[var(--color-separator)] bg-black/10 text-[var(--color-label-secondary)] hover:border-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] transition-colors"
                      }
                    >
                      {id}
                    </button>
                  );
                })}
                {(cloudModels[selectedCloud] ?? []).length === 0 && (
                  <span className="text-[12px] text-[var(--color-label-tertiary)]">
                    {t("ui.no-models-returned-enter-one-manually-below", "No models returned — enter one manually below.")}
                  </span>
                )}
              </div>
            )}
            <form
              className="flex w-full gap-2"
              onSubmit={(event) => {
                event.preventDefault();
                const model = (manualCloudModel[selectedCloud] ?? "").trim();
                if (model) void selectCloudModel(selectedCloud, model);
              }}
            >
              <input
                value={manualCloudModel[selectedCloud] ?? ""}
                onChange={(event) => setManualCloudModel((m) => ({ ...m, [selectedCloud]: event.target.value }))}
                placeholder={t("ui.model-name", "Model name")}
                aria-label={t("ui.custom-model-name", "Custom model name")}
                className="min-w-0 flex-1 px-3 py-1.5 rounded-lg text-[12px] border border-[var(--color-separator)] bg-black/10 text-[var(--color-label-primary)] placeholder:text-[var(--color-label-tertiary)]"
              />
              <button
                type="submit"
                disabled={!(manualCloudModel[selectedCloud] ?? "").trim()}
                className="px-3 py-1.5 rounded-lg text-[12px] font-medium border border-[var(--color-accent)] text-[var(--color-accent)] disabled:opacity-40"
              >
                {t("ui.use-model", "Use model")}
              </button>
            </form>
          </div>
        )}

        {/* Windows-only: sd.cpp GPU build status. Metal on macOS is
            compiled into the binary, no provisioning needed — the
            bundled Windows CPU build always works, but a vendor-matched
            GPU build (fetched on demand) renders in seconds instead of
            minutes. */}
        {isWindows && gpuStatus?.platform_supported && (
          <div className="mb-4 surface p-3 flex items-center justify-between gap-3">
            <div className="flex items-center gap-3 min-w-0">
              <Cpu className={`w-4 h-4 shrink-0 ${gpuStatus.image_accelerated ? "text-[var(--color-system-green)]" : "text-[var(--color-label-tertiary)]"}`} />
              <div className="min-w-0">
                <div className="text-[13px] font-medium text-[var(--color-label-primary)]">
                  {t("settings.image.gpu_title", "GPU acceleration")}
                </div>
                <div className="text-[11px] text-[var(--color-label-tertiary)]">
                  {gpuStatus.vendor === "other"
                    ? t("settings.image.gpu_none", "No supported GPU detected — running on CPU.")
                    : gpuStatus.image_accelerated
                      ? t("settings.image.gpu_ready", "Accelerated ({{vendor}})", { vendor: gpuStatus.vendor })
                      : t("settings.image.gpu_not_ready", "{{vendor}} detected — not yet accelerated", { vendor: gpuStatus.vendor })}
                </div>
              </div>
            </div>
            {gpuStatus.vendor !== "other" && !gpuStatus.image_accelerated && (
              <Button size="sm" variant="ghost" onClick={handleGpuRetry} disabled={gpuRetrying}>
                {gpuRetrying ? (
                  <><Loader2 className="w-3.5 h-3.5 animate-spin mr-1" />{t("settings.image.gpu_retrying", "Setting up…")}</>
                ) : (
                  t("settings.image.gpu_retry", "Retry GPU setup")
                )}
              </Button>
            )}
          </div>
        )}

        {/* Active model banner */}
        {activeModel && (
          <div className="mb-4 p-3 rounded-lg bg-[var(--color-accent-soft)] border border-[var(--color-accent)]/20 flex items-center gap-2.5">
            <Image className="w-4 h-4 text-[var(--color-accent)] shrink-0" />
            <div className="text-xs flex-1">
              <span className="text-[var(--color-label-secondary)]">
                {t("settings.image.active_model", "Active image model:")}
              </span>{" "}
              <span className="font-medium text-[var(--color-label-primary)]">{activeModel}</span>
              {readyModels.some((m) => m.id === activeModel) ? (
                <span className="ml-2 text-[var(--color-system-green)]">● {t("settings.image.ready", "Ready")}</span>
              ) : (
                <span className="ml-2 text-[var(--color-warm)]">● {t("settings.image.not_installed", "Not installed - download it below")}</span>
              )}
            </div>
          </div>
        )}

        {/* Quick test */}
        <div className="flex items-center gap-3 mb-4">
          <Button size="sm" variant="ghost" onClick={handleTest} disabled={testState === "running"} aria-label={t("settings.image.test", "Test image generation")}>
            {testState === "running" ? (
              <><Loader2 className="w-3.5 h-3.5 animate-spin mr-1" />{t("settings.image.testing", "Testing...")}</>
            ) : (
              t("settings.image.test", "Test image generation")
            )}
          </Button>
          {testState === "ok" && <span role="status" tabIndex={0} className="text-xs text-[var(--color-system-green)]">✓ {t("settings.image.works", "Works")}</span>}
          {testState === "fail" && <span role="alert" tabIndex={0} className="text-xs text-red-400">✗ {t("settings.image.test_failed_short", "Failed - install a model below")}</span>}
        </div>
      </div>

      <div className="surface p-4 space-y-4">
        <div>
          <h3 className="text-[12px] font-semibold uppercase tracking-wider text-[var(--color-label-secondary)]">
            {t("settings.image.story_images", "Story Images")}
          </h3>
          <p className="text-[11px] text-[var(--color-label-tertiary)] mt-1">
            {t("settings.image.story_images_desc", "Control when scenes are illustrated automatically. Manual illustration remains available in the story.")}
          </p>
        </div>
        <div className="grid grid-cols-2 gap-3">
          <label className="text-[11px] text-[var(--color-label-secondary)]">
            {t("settings.image.automatic", "Automatic illustrations")}
            <select value={imageFrequency} onChange={(e) => updatePreference("story_image_frequency", e.target.value)} className="mt-1 w-full rounded-lg bg-[var(--color-fill-tertiary)] text-[12px] text-[var(--color-label-primary)] px-2.5 py-2 outline-none">
              <option value="off">{t("ui.off", "Off")}</option>
              <option value="first">{t("ui.first-scene-only", "First scene only")}</option>
              <option value="every">{t("ui.every-scene", "Every scene")}</option>
              <option value="every2">{t("ui.every-2-turns", "Every 2 turns")}</option>
              <option value="every3">{t("ui.every-3-turns", "Every 3 turns")}</option>
              <option value="important">{t("ui.important-scenes-only", "Important scenes only")}</option>
            </select>
          </label>
          <label className="text-[11px] text-[var(--color-label-secondary)]">
            {t("settings.image.style", "Default visual style")}
            <select value={imageStyle} onChange={(e) => updatePreference("story_image_style", e.target.value)} className="mt-1 w-full rounded-lg bg-[var(--color-fill-tertiary)] text-[12px] text-[var(--color-label-primary)] px-2.5 py-2 outline-none">
              <option value="cinematic">{t("ui.cinematic", "Cinematic")}</option>
              <option value="anime">{t("ui.anime", "Anime")}</option>
              <option value="realistic">{t("ui.realistic", "Realistic")}</option>
              <option value="watercolor">{t("ui.watercolor", "Watercolor")}</option>
              <option value="ink">{t("ui.ink", "Ink")}</option>
              <option value="dark-fantasy">{t("ui.dark-fantasy", "Dark Fantasy")}</option>
              <option value="manga">{t("ui.manga", "Manga")}</option>
            </select>
          </label>
          <label className="text-[11px] text-[var(--color-label-secondary)]">
            {t("settings.image.quality", "Image quality")}
            <select value={imageQuality} onChange={(e) => updatePreference("story_image_quality", e.target.value)} className="mt-1 w-full rounded-lg bg-[var(--color-fill-tertiary)] text-[12px] text-[var(--color-label-primary)] px-2.5 py-2 outline-none">
              <option value="draft">{t("ui.draft-faster", "Draft · faster")}</option>
              <option value="balanced">{t("ui.balanced", "Balanced")}</option>
              <option value="high">{t("ui.high-quality-slower", "High quality · slower")}</option>
            </select>
          </label>
          <label className="text-[11px] text-[var(--color-label-secondary)]">
            {t("settings.image.max_auto", "Max automatic images")}
            <select value={imageMaxAuto} onChange={(e) => updatePreference("story_image_max_auto", e.target.value)} className="mt-1 w-full rounded-lg bg-[var(--color-fill-tertiary)] text-[12px] text-[var(--color-label-primary)] px-2.5 py-2 outline-none">
              <option value="1">1 per story</option>
              <option value="3">3 per story</option>
              <option value="10">10 per story</option>
              <option value="0">{t("ui.unlimited", "Unlimited")}</option>
            </select>
          </label>
        </div>
      </div>

      {/* Installed / available SD models */}
      <div>
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-[12px] font-semibold uppercase tracking-wider text-[var(--color-label-secondary)]">
            {t("settings.image.catalog", "Stable Diffusion Models")}
          </h3>
          <button onClick={refresh} aria-label={t("settings.image.refresh", "Refresh image models")} className="text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] transition-colors">
            <RefreshCw className="w-3.5 h-3.5" />
          </button>
        </div>

        <div className="mb-3 p-3 rounded-lg bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)]">
          <p className="text-xs text-[var(--color-label-secondary)]">
            <HardDrive className="w-3.5 h-3.5 inline mr-1" />
            <span className="font-medium">stable-diffusion.cpp</span>
            {" — "}{t("settings.image.sdcpp_status", "Bundled with FPV. Download a model to enable image generation.")}
          </p>
        </div>

        {loading ? (
          <div role="status" tabIndex={0} className="flex items-center gap-2 text-[var(--color-label-tertiary)] text-sm py-4">
            <Loader2 className="w-4 h-4 animate-spin" />
            {t("settings.image.loading", "Loading image models...")}
          </div>
        ) : loadError ? (
          <div role="alert" tabIndex={0} className="flex items-center justify-between gap-3 rounded-lg border border-red-500/20 bg-red-500/10 p-3 text-sm text-red-300">
            <span>{t("settings.image.load_error", "Could not load image models.")} {loadError}</span>
            <Button size="sm" variant="ghost" onClick={refresh}>{t("settings.models.retry", "Retry")}</Button>
          </div>
        ) : (
          <div className="space-y-2">
            {models.length === 0 && (
              <p className="text-[var(--color-label-tertiary)] text-sm py-2">
                {t("settings.image.empty_catalog", "No image models in catalog. Check your connection and refresh.")}
              </p>
            )}
            {models.map((m) => {
              const isActive = activeModel === m.id;
              return (
                <div key={m.id} className="surface px-4 py-3 flex items-center justify-between">
                  <div className="flex items-center gap-3 min-w-0">
                    <Image className="w-4 h-4 text-[var(--color-label-tertiary)] shrink-0" />
                    <div className="min-w-0">
                      <span className="text-sm text-[var(--color-label-primary)]">{m.id}</span>
                      <span className="text-xs text-[var(--color-label-tertiary)] ml-2">
                        {m.ready
                          ? `${(m.download_gb ?? 0) > 0 ? "~" : ""}${m.download_gb} GB`
                          : `~${m.download_gb} GB to download`}
                        {m.large && ` · ${t("settings.image.large", "large")}`}
                      </span>
                      {!m.ram_ok && (
                        <p className="text-xs text-[var(--color-warm)]">
                          {t("settings.image.needs_ram", "Needs {{gb}} GB RAM - below your hardware tier.", { gb: m.min_ram_gib })}
                        </p>
                      )}
                      {isActive && (
                        <p className="text-xs text-[var(--color-accent)] font-medium">● {t("settings.image.active", "Active")}</p>
                      )}
                    </div>
                  </div>
                  <div className="flex items-center gap-1.5 shrink-0">
                    {m.ready && !isActive && (
                      <button
                        onClick={() => handleSetDefault(m.id)}
                        className="text-[11px] text-[var(--color-accent)] hover:underline transition-colors"
                      >
                        {t("settings.image.use", "Use")}
                      </button>
                    )}
                    {busy === m.id ? (
                      <span className="flex items-center gap-1.5 text-xs text-[var(--color-accent)]">
                        <Loader2 className="w-3.5 h-3.5 animate-spin" />
                        {downloadPct != null ? `${downloadPct}%` : "…"}
                      </span>
                    ) : m.ready ? (
                      <Button size="sm" variant="ghost" onClick={() => handleDelete(m.id)} disabled={busy !== null || isActive} aria-label={t("settings.image.delete", "Delete {{id}}", { id: m.id })}>
                        <XCircle aria-hidden="true" className="w-3.5 h-3.5 text-[var(--color-label-tertiary)] hover:text-red-400" />
                      </Button>
                    ) : (
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => handleDownload(m.id)}
                        disabled={busy !== null}
                        aria-label={t("settings.models.install", "Install {{id}}", { id: m.id })}
                      >
                        <Download aria-hidden="true" className="w-3.5 h-3.5" />
                      </Button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* In-progress weight download */}
      {busy && (
        <div className="p-3 rounded-lg bg-[var(--color-accent-soft)] border border-[var(--color-accent)]/20">
          <div className="flex items-center gap-2 text-sm text-[var(--color-accent)]">
            <Loader2 className="w-4 h-4 animate-spin" />
            {t("settings.image.downloading", "Downloading {{model}} weights...", { model: activeModel })}
          </div>
          {downloadPct != null && (
            <div className="mt-2 text-xs text-[var(--color-label-tertiary)]">{downloadPct}%</div>
          )}
          <div className="mt-1.5 h-1 bg-[var(--color-fill-primary)] rounded-full overflow-hidden">
            <div
              className="h-full bg-[var(--color-accent)] transition-[width] duration-300"
              style={{ width: downloadPct != null ? `${Math.min(100, downloadPct)}%` : "33%" }}
            />
          </div>
          <button onClick={handleCancelDownload} aria-label={t("settings.image.cancel_download", "Cancel download")} className="mt-2 text-[11px] text-[var(--color-label-tertiary)] hover:text-red-400">
            {t("settings.image.cancel_download", "Cancel download")}
          </button>
        </div>
      )}
    </div>
  );
}
