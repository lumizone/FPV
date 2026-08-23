import { useEffect, useState } from "react";
import {
  Download,
  Trash2,
  Check,
  RefreshCw,
  HardDrive,
  Loader2,
  Zap,
  XCircle,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  modelList,
  modelDelete,
  modelSetDefault,
  byokList,
  codexStatus,
  codexLogin,
  codexModelGet,
  codexModelSet,
  cloudListModels,
  type ModelListResult,
} from "@/lib/tauri";
import { useApp } from "@/lib/store";
import { cn } from "@/lib/utils";
import { HfBrowser } from "./HfBrowser";
import { Cloud, CheckCircle2 } from "lucide-react";

const CLOUD_PROVIDER_LABELS: Record<string, string> = {
  xai: "xAI (Grok)",
  together: "Together.ai",
  fireworks: "Fireworks.ai",
  novita: "Novita AI",
  siliconflow: "SiliconFlow",
  moonshot: "Moonshot AI (Kimi)",
  zhipu: "Zhipu (GLM)",
  qwen: "Qwen (Alibaba)",
  baichuan: "Baichuan",
  minimax: "MiniMax",
  stepfun: "StepFun",
  modelscope: "ModelScope",
  xiaomimimo: "Xiaomi MiMo",
  doubao: "Doubao (ByteDance)",
  hunyuan: "Tencent Hunyuan",
  upstage: "Upstage (Solar)",
  yi: "01.AI (Yi)",
  plamo: "PLaMo (Preferred Networks)",
  nvidia: "NVIDIA NIM",
  cohere: "Cohere",
  cerebras: "Cerebras",
  sambanova: "SambaNova",
  perplexity: "Perplexity",
  ai21: "AI21 (Jamba)",
  venice: "Venice.ai",
};

const CLOUD_PROVIDER_IDS = new Set([
  "openai", "anthropic", "deepseek", "google", "openrouter", "mistral", "groq", "xai",
  "together", "fireworks", "novita", "siliconflow", "moonshot", "zhipu", "qwen",
  "baichuan", "minimax", "stepfun", "modelscope", "xiaomimimo",
  "doubao", "hunyuan", "upstage", "yi", "plamo",
  "nvidia", "cohere", "cerebras", "sambanova", "perplexity", "ai21",
  "venice",
  "custom", "codex",
]);

/// Settings tab for the LLM chat/narration model. One place to pick the
/// narrator — local (Ollama) OR any cloud provider with a saved BYOK key.
/// Styled after Local Waifu's Models tab (surface cards + uppercase
/// section headings + hardware chip). Narrative and image models stay
/// SEPARATE tabs by explicit user request.
export function NarrativeModelTab() {
  const { t } = useTranslation();
  const hardware = useApp((s) => s.hardware);
  const [data, setData] = useState<ModelListResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [configuredProviders, setConfiguredProviders] = useState<string[]>([]);
  const [manualModel, setManualModel] = useState<Record<string, string>>({});
  const [cloudModels, setCloudModels] = useState<Record<string, string[]>>({});
  const [cloudModelsLoading, setCloudModelsLoading] = useState<Record<string, boolean>>({});
  const [cloudModelsError, setCloudModelsError] = useState<Record<string, string | null>>({});
  const [codexReady, setCodexReady] = useState<boolean | null>(null);
  const [codexSigningIn, setCodexSigningIn] = useState(false);
  const [codexModel, setCodexModel] = useState("gpt-5.6-terra");
  const startDownload = useApp((s) => s.startModelDownload);
  const cancelDownload = useApp((s) => s.cancelModelDownload);
  const modelDownload = useApp((s) => s.model_download);
  const selectedModel = useApp((s) => s.selected_model);
  const setSelectedModel = useApp((s) => s.setSelectedModel);
  const semanticMemoryEnabled = useApp((s) => s.preferences["semanticMemoryEnabled"] === "true");

  const refresh = () => {
    setLoading(true);
    setLoadError(null);
    Promise.all([modelList(), byokList(), codexStatus().catch(() => false), codexModelGet().catch(() => "gpt-5.6-terra")])
      .then(([r, keys, codex, savedCodexModel]) => {
        setData(r);
        setConfiguredProviders(keys.providers);
        setCodexReady(codex);
        setCodexModel(savedCodexModel);
      })
      .catch((e) =>
        setLoadError((e as { message?: string })?.message ?? t("settings.models.load_error", "Could not load models."))
      )
      .finally(() => setLoading(false));
  };

  useEffect(() => { refresh(); }, []);

  // Live model discovery — fetched directly from each configured provider's
  // own API, never a hand-maintained list (providers ship new models faster
  // than this app can be updated to know their names).
  const fetchCloudModels = (providerId: string) => {
    setCloudModelsLoading((s) => ({ ...s, [providerId]: true }));
    setCloudModelsError((s) => ({ ...s, [providerId]: null }));
    cloudListModels(providerId)
      .then((ids) => setCloudModels((s) => ({ ...s, [providerId]: ids })))
      .catch((e: any) =>
        setCloudModelsError((s) => ({
          ...s,
          [providerId]: (e as { message?: string })?.message ?? t("settings.models.cloud_list_error", "Could not load models."),
        }))
      )
      .finally(() => setCloudModelsLoading((s) => ({ ...s, [providerId]: false })));
  };

  const configuredCloudProvidersKey = configuredProviders
    .filter((id) => CLOUD_PROVIDER_IDS.has(id) && id !== "codex")
    .join(",");

  useEffect(() => {
    configuredCloudProvidersKey
      .split(",")
      .filter(Boolean)
      .forEach((id) => fetchCloudModels(id));
    // Re-fetch only when the SET of configured cloud providers changes, not
    // on every render (fetchCloudModels is stable in intent but recreated
    // each render, so it can't be a dep without refetching constantly).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [configuredCloudProvidersKey]);

  useEffect(() => {
    if (!codexSigningIn) return;
    const timer = window.setInterval(() => {
      codexStatus().then((ready) => {
        if (ready) {
          setCodexReady(true);
          setCodexSigningIn(false);
        }
      }).catch(() => {});
    }, 2_000);
    return () => window.clearInterval(timer);
  }, [codexSigningIn]);

  const handleDelete = async (model: string) => {
    if (!window.confirm(t("settings.models.delete_confirm", "Delete model \"{{name}}\"? This cannot be undone.", { name: model }))) return;
    setDeleting(model);
    try {
      await modelDelete(model);
      refresh();
    } catch (e: any) {
      console.error("delete failed", e);
      toast.error(t("settings.models.delete_failed", "Failed to delete model"));
    } finally {
      setDeleting(null);
    }
  };

  const handleInstall = (id: string) => {
    startDownload(id, "chat");
  };

  const handleInstallEmbed = (id: string) => {
    startDownload(id, "embed");
  };

  const handleSetDefault = async (model: string) => {
    try {
      await modelSetDefault("chat", model);
      setSelectedModel(model);
      refresh();
    } catch (e: any) {
      console.error("set default failed", e);
      toast.error(t("settings.models.set_default_failed", "Failed to set default model"));
    }
  };

  const handleSetEmbedDefault = async (model: string) => {
    try {
      await modelSetDefault("embed", model);
      refresh();
    } catch (e: any) {
      console.error("set embed default failed", e);
      toast.error(t("settings.models.set_default_failed", "Failed to set default model"));
    }
  };

  // Persist the Codex model name (no app update needed when OpenAI ships a
  // newer model — the name is passed through to the Codex CLI verbatim).
  const saveCodexModel = async (value: string) => {
    const model = value.trim();
    if (!model || model === codexModel) return;
    try {
      await codexModelSet(model);
      setCodexModel(model);
    } catch (e: any) {
      console.error("save codex model failed", e);
      toast.error(t("settings.models.set_default_failed", "Failed to set default model"));
    }
  };

  const chatModels = (data?.catalog ?? []).filter((m) => m.kind === "chat");
  const embedModels = (data?.catalog ?? []).filter((m) => m.kind === "embed");
  const installedEmbed = (data?.installed ?? []).filter((m) => embedModels.some((c) => c.id === m.name || m.name.startsWith(c.id + ":")));
  const installedChat = (data?.installed ?? []).filter((m) => !installedEmbed.some((e) => e.name === m.name));
  const availableEmbeds = embedModels.filter(
    (c) => !installedEmbed.some((e) => e.name === c.id || e.name.startsWith(c.id + ":"))
  );

  return (
    <div className="space-y-6">
      {/* Active narrator + embedding banner */}
      {(() => {
        const active =
          [selectedModel, data?.user_chat_default, data?.hardware_chat_default]
            .find((model) => model && model.split(":")[0] !== "fal") ?? null;
        const activeProvider = active?.split(":")[0];
        const isCloud = active ? !!activeProvider && CLOUD_PROVIDER_IDS.has(activeProvider) : false;
        const embedActive = semanticMemoryEnabled && installedEmbed.length > 0;
        return (
          <div className="p-4 rounded-xl border border-[var(--color-accent)]/25 bg-[var(--color-accent-soft)] space-y-2.5">
            {/* Narrator row */}
            <div className="flex items-center gap-3">
              <CheckCircle2 className="w-5 h-5 text-[var(--color-accent)] shrink-0" />
              <div className="min-w-0 flex-1">
                <div className="text-[11px] uppercase tracking-wider text-[var(--color-label-tertiary)]">
                  {t("settings.models.active_narrator", "Active Narrator")}
                </div>
                <div className="text-[14px] font-semibold text-[var(--color-label-primary)] truncate">
                  {active ?? "—"}
                </div>
              </div>
              <span
                className={`shrink-0 text-[10px] font-semibold uppercase tracking-wide px-2 py-1 rounded-full ${
                  isCloud
                    ? "bg-[var(--color-magic-soft)] text-[var(--color-magic)]"
                    : "bg-[var(--color-system-green)]/10 text-[var(--color-system-green)]"
                }`}
              >
                {isCloud ? "Cloud" : "Local"}
              </span>
            </div>
            {/* Embedding row */}
            <div className="flex items-center gap-3 border-t border-[var(--color-accent)]/10 pt-2.5">
              <CheckCircle2 className={`w-4 h-4 shrink-0 ${embedActive ? "text-[var(--color-system-green)]" : "text-[var(--color-label-quaternary)]"}`} />
              <div className="min-w-0 flex-1">
                <div className="text-[11px] uppercase tracking-wider text-[var(--color-label-tertiary)]">
                  {t("ui.embedding", "Embedding")}
                </div>
                <div className="text-[13px] font-medium text-[var(--color-label-secondary)] truncate">
                  {installedEmbed.length === 0 ? "Not installed" : embedActive ? installedEmbed[0].name : "Disabled"}
                </div>
              </div>
              <span className={`shrink-0 text-[10px] font-semibold uppercase tracking-wide px-2 py-1 rounded-full ${
                embedActive ? "bg-[var(--color-system-green)]/10 text-[var(--color-system-green)]" : "bg-[var(--color-fill-primary)] text-[var(--color-label-tertiary)]"
              }`}>
                {embedActive ? "Active" : "Inactive"}
              </span>
            </div>
          </div>
        );
      })()}

      {/* Hardware summary chip */}
      <div className="surface p-4 flex flex-col items-center justify-center text-center space-y-2 mb-2">
        <div className="flex items-center gap-2 text-[var(--color-system-green)] bg-[var(--color-system-green)]/10 px-3 py-1.5 rounded-full text-[12px] font-medium mb-1">
          <Zap className="w-4 h-4" />
          {hardware ? "Hardware Detected" : "Detecting hardware…"}
        </div>
        <div className="text-[13px] text-[var(--color-label-primary)] font-medium">
          {hardware ? `${hardware.chip} · ${hardware.ram_gb} GB Unified Memory` : "—"}
        </div>
        <div className="text-[12px] text-[var(--color-label-secondary)] max-w-[340px]">
          {hardware
            ? `Recommended narrator: ${hardware.recommended_chat_model} (${hardware.tier} tier). Runs entirely on this machine.`
            : "Hardware info loading…"}
        </div>
      </div>

      {/* Installed models */}
      <div>
        <h3 className="text-[12px] font-semibold uppercase tracking-wider text-[var(--color-label-secondary)] mb-3">
          {t("settings.models.installed", "Installed Models")}
        </h3>
        {loading ? (
          <div className="flex items-center gap-2 text-[var(--color-label-tertiary)] text-sm py-4">
            <Loader2 className="w-4 h-4 animate-spin" />
            {t("ui.loading", "Loading...")}
          </div>
        ) : loadError ? (
          <div role="alert" tabIndex={0} className="flex items-center justify-between gap-3 rounded-lg border border-red-500/20 bg-red-500/10 p-3 text-sm text-red-300">
            <span>{t("settings.models.load_error", "Could not load models.")} {loadError}</span>
            <Button size="sm" variant="ghost" onClick={refresh}>{t("settings.models.retry", "Retry")}</Button>
          </div>
        ) : (data?.installed ?? []).length === 0 ? (
          <p className="text-[var(--color-label-tertiary)] text-sm py-2">
            {t("settings.models.no_models", "No models installed. Pull one from the catalog below.")}
          </p>
        ) : (
          <div className="space-y-4">
            {/* Embedding models — semantic memory. Multiple open-source
                options; the active one is the embed default. */}
            {embedModels.length > 0 && (
              <div>
                <div className="text-[10px] uppercase tracking-[0.1em] text-[var(--color-label-tertiary)] mb-1.5 px-1">
                  {t("ui.embedding", "Embedding")}
                </div>
                {installedEmbed.map((m) => {
                  const entry = embedModels.find((c) => c.id === m.name || m.name.startsWith(c.id + ":"));
                  return (
                    <div key={m.name} className="surface px-4 py-3 flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <Check className="w-4 h-4 text-[var(--color-system-green)] shrink-0" />
                        <div>
                          <span className="text-[14px] font-medium text-[var(--color-label-primary)]">
                            {m.name}
                          </span>
                          {entry?.recommended && (
                            <span className="ml-2 text-[10px] font-semibold uppercase tracking-wide px-1.5 py-0.5 rounded-full bg-[var(--color-accent)]/15 text-[var(--color-accent)]">
                              Recommended
                            </span>
                          )}
                          <span className="text-[12px] text-[var(--color-label-tertiary)] ml-2">
                            {(m.size / 1_000_000_000).toFixed(1)} GB
                          </span>
                        </div>
                      </div>
                      <div className="flex items-center gap-2">
                        {m.name !== data?.user_embed_default ? (
                          <button
                            onClick={() => handleSetEmbedDefault(m.name)}
                            className="text-[12px] text-[var(--color-accent)] hover:underline transition-colors"
                          >
                            {t("settings.models.set_default", "Set default")}
                          </button>
                        ) : (
                          <span className="text-[12px] text-[var(--color-accent)]/70 font-medium">
                            {t("ui.active", "Active")}
                          </span>
                        )}
                        <button
                          onClick={() => handleDelete(m.name)}
                          disabled={deleting === m.name}
                          aria-label={t("ui.delete-model", "Delete {{name}}", { name: m.name })}
                          className="text-[var(--color-label-tertiary)] hover:text-red-400 transition-colors disabled:opacity-30"
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    </div>
                  );
                })}
                {availableEmbeds.map((c) => {
                  const isDownloading = modelDownload?.phase === "active" && modelDownload.id === c.id;
                  return (
                    <div key={c.id} className="surface px-4 py-3 flex items-center justify-between">
                      <div className="flex items-center gap-3 min-w-0">
                        <HardDrive className="w-4 h-4 text-[var(--color-label-secondary)] shrink-0" />
                        <div className="min-w-0">
                          <span className="text-[14px] font-medium text-[var(--color-label-primary)]">
                            {c.id}
                          </span>
                          {c.recommended && (
                            <span className="ml-2 text-[10px] font-semibold uppercase tracking-wide px-1.5 py-0.5 rounded-full bg-[var(--color-accent)]/15 text-[var(--color-accent)]">
                              Recommended
                            </span>
                          )}
                          {c.description && (
                            <p className="text-[12px] text-[var(--color-label-tertiary)] truncate">
                              {c.description}
                            </p>
                          )}
                        </div>
                      </div>
                      {isDownloading ? (
                        <Button size="sm" variant="ghost" onClick={() => cancelDownload()} className="text-red-400 hover:text-red-300">
                          <XCircle className="w-3.5 h-3.5 mr-1" />
                          {t("ui.stop", "Stop")}
                        </Button>
                      ) : (
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => handleInstallEmbed(c.id)}
                          disabled={modelDownload?.phase === "active"}
                          aria-label={t("settings.models.install", "Install {{id}}", { id: c.id })}
                        >
                          <Download className="w-3.5 h-3.5" />
                        </Button>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
            {/* Narrator models */}
            {installedChat.length > 0 && (
              <div>
                <div className="text-[10px] uppercase tracking-[0.1em] text-[var(--color-label-tertiary)] mb-1.5 px-1">
                  {t("ui.narrator", "Narrator")}
                </div>
                {installedChat.map((m) => (
                  <div key={m.name} className="surface px-4 py-3 flex items-center justify-between">
                    <div className="flex items-center gap-3">
                      <Check className="w-4 h-4 text-[var(--color-accent)] shrink-0" />
                      <div>
                        <span className="text-[14px] font-medium text-[var(--color-label-primary)]">
                          {m.name}
                        </span>
                        <span className="text-[12px] text-[var(--color-label-tertiary)] ml-2">
                          {(m.size / 1_000_000_000).toFixed(1)} GB
                        </span>
                      </div>
                    </div>
                    <div className="flex items-center gap-2">
                      {m.name !== selectedModel ? (
                        <button
                          onClick={() => handleSetDefault(m.name)}
                          className="text-[12px] text-[var(--color-accent)] hover:underline transition-colors"
                        >
                          {t("settings.models.set_default", "Set default")}
                        </button>
                      ) : (
                        <span className="text-[12px] text-[var(--color-accent)]/70 font-medium">
                          {t("ui.active", "Active")}
                        </span>
                      )}
                      <button
                        onClick={() => handleDelete(m.name)}
                        disabled={deleting === m.name}
                        aria-label={t("ui.delete-model", "Delete {{name}}", { name: m.name })}
                        className="text-[var(--color-label-tertiary)] hover:text-red-400 transition-colors disabled:opacity-30"
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Model catalog */}
      <div>
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-[12px] font-semibold uppercase tracking-wider text-[var(--color-label-secondary)]">
            {t("settings.models.catalog", "Available Models")}
          </h3>
          <button onClick={refresh} aria-label={t("ui.refresh-model-catalog", "Refresh model catalog")} className="text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] transition-colors">
            <RefreshCw className="w-3.5 h-3.5" />
          </button>
        </div>
        <div className="space-y-2">
          {chatModels.map((m) => {
            const isInstalled = data?.installed.some(
              (i) => i.name === m.id || i.name.startsWith(m.id + ":")
            );
            const isDownloading = modelDownload?.phase === "active" && modelDownload.id === m.id;
            return (
              <div key={m.id} className="surface px-4 py-3 flex items-center justify-between">
                <div className="flex items-center gap-3 min-w-0">
                  <HardDrive className="w-4 h-4 text-[var(--color-label-secondary)] shrink-0" />
                  <div className="min-w-0">
                    <span className="text-[14px] font-medium text-[var(--color-label-primary)]">
                      {m.id}
                    </span>
                    {m.description && (
                      <p className="text-[12px] text-[var(--color-label-tertiary)] truncate">
                        {m.description}
                      </p>
                    )}
                  </div>
                </div>
                {isDownloading ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => cancelDownload()}
                    className="text-red-400 hover:text-red-300"
                  >
                    <XCircle className="w-3.5 h-3.5 mr-1" />
                    {t("ui.stop", "Stop")}
                  </Button>
                ) : (
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => handleInstall(m.id)}
                    disabled={modelDownload?.phase === "active" || isInstalled}
                  >
                    {isInstalled
                      ? <Check className="w-3.5 h-3.5 text-[var(--color-accent)]" />
                      : <Download className="w-3.5 h-3.5" />
                    }
                  </Button>
                )}
              </div>
            );
          })}
          {chatModels.length === 0 && !loading && (
            <p className="text-[var(--color-label-tertiary)] text-sm py-2">
              {t("ui.no-models-in-catalog-check-your-ollama-connectio", "No models in catalog. Check your Ollama connection.")}
            </p>
          )}
        </div>
      </div>

      {/* Cloud narrator — one place to pick a cloud LLM alongside local */}
      <div>
        <h3 className="text-[12px] font-semibold uppercase tracking-wider text-[var(--color-label-secondary)] mb-3">
          {t("ui.codex-narrator", "Codex Narrator")}
        </h3>
        <div className="surface px-4 py-3 space-y-2.5">
          <div className="flex items-center justify-between gap-3">
            <div>
              <div className="text-[13px] font-medium text-[var(--color-label-primary)]">{t("ui.chatgpt-plus-pro-via-codex", "ChatGPT Plus / Pro via Codex")}</div>
              <p className="text-[12px] text-[var(--color-label-tertiary)] mt-0.5">
                Uses the local Codex CLI session. Story context is sent to Codex; unavailable for Local only worlds.
              </p>
            </div>
            <span className={`shrink-0 text-[10px] font-semibold uppercase tracking-wide px-2 py-1 rounded-full ${
              codexReady === true ? "bg-[var(--color-system-green)]/10 text-[var(--color-system-green)]" : "bg-[var(--color-fill-primary)] text-[var(--color-label-tertiary)]"
            }`}>
              {codexReady === null ? "Checking" : codexReady ? "Connected" : "Sign in required"}
            </span>
          </div>
          <div className="flex items-center gap-2">
            <Button
              size="sm"
              variant="ghost"
              disabled={!codexReady}
              onClick={() => handleSetDefault(`codex:${codexModel}`)}
            >
              {selectedModel?.startsWith("codex:") ? "Active" : "Use Codex"}
            </Button>
            {!codexReady && (
              <Button
                size="sm"
                variant="ghost"
                disabled={codexSigningIn}
                onClick={() => {
                  setCodexSigningIn(true);
                  codexLogin().catch(() => setCodexSigningIn(false));
                }}
              >
                {codexSigningIn ? "Waiting for sign in..." : "Sign in with ChatGPT"}
              </Button>
            )}
          </div>
          <div className="flex items-center gap-2">
            <input
              value={codexModel}
              onChange={(event) => setCodexModel(event.target.value)}
              onBlur={() => saveCodexModel(codexModel)}
              onKeyDown={(event) => {
                if (event.key === "Enter") saveCodexModel(codexModel);
              }}
              placeholder="Model name"
              aria-label="Codex model name"
              className="min-w-0 flex-1 px-3 py-1.5 rounded-lg text-[12px] border border-[var(--color-separator)] bg-black/10 text-[var(--color-label-primary)] placeholder:text-[var(--color-label-tertiary)]"
            />
            <span className="shrink-0 text-[11px] text-[var(--color-label-tertiary)]">
              Model passed to the Codex CLI — edit any time, no app update needed.
            </span>
          </div>
        </div>
      </div>

      {/* Cloud narrator — one place to pick a cloud LLM alongside local */}
      <div>
        <h3 className="text-[12px] font-semibold uppercase tracking-wider text-[var(--color-label-secondary)] mb-3">
          {t("settings.models.cloud_narrator", "Cloud Narrator")}
        </h3>
        <p className="text-[12px] text-[var(--color-label-tertiary)] mb-3">
          {t(
            "settings.models.cloud_hint",
            "Pick a cloud model to run the narrator on your key. Add or manage keys in Settings → AI Cloud Connection."
          )}
        </p>
        {configuredProviders.length === 0 ? (
          <div className="surface px-4 py-4 flex items-center gap-3">
            <Cloud className="w-4 h-4 text-[var(--color-label-tertiary)] shrink-0" />
            <div className="text-[12px] text-[var(--color-label-secondary)]">
              No cloud keys saved yet. Go to{" "}
              <span className="text-[var(--color-label-primary)] font-medium">
                {t("ui.settings-ai-cloud-connection", "Settings → AI Cloud Connection")}
              </span>{" "}
              to add one.
            </div>
          </div>
        ) : (
          <div className="space-y-3">
            {configuredProviders.filter((providerId) => CLOUD_PROVIDER_IDS.has(providerId)).map((providerId) => {
              const models = cloudModels[providerId] ?? [];
              const modelsLoading = cloudModelsLoading[providerId];
              const modelsError = cloudModelsError[providerId];
              const anyMatch = models.some(
                (id) => selectedModel === `${providerId}:${id}`
              );
              return (
                <div key={providerId} className="surface px-4 py-3">
                  <div className="flex items-center gap-2 mb-2">
                    <img
                      src={`/icons/${providerId}.svg`}
                      alt=""
                      className="w-5 h-5 shrink-0 rounded"
                      onError={(e) => { const image = e.currentTarget; if (!image.src.endsWith("/icons/provider.svg")) image.src = "/icons/provider.svg"; }}
                    />
                    <span className="text-[13px] font-medium text-[var(--color-label-primary)] capitalize">
                      {CLOUD_PROVIDER_LABELS[providerId] ?? providerId.replace(/_/g, " ")}
                    </span>
                    {anyMatch && (
                      <span className="text-[10px] font-semibold text-[var(--color-accent)] bg-[var(--color-accent)]/10 px-2 py-0.5 rounded-full uppercase tracking-wide">
                        {t("ui.active", "Active")}
                      </span>
                    )}
                    <button
                      onClick={() => fetchCloudModels(providerId)}
                      disabled={modelsLoading}
                      aria-label={t("ui.refresh-model-catalog", "Refresh model catalog")}
                      className="ml-auto text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] transition-colors disabled:opacity-40"
                    >
                      <RefreshCw className={cn("w-3.5 h-3.5", modelsLoading && "animate-spin")} />
                    </button>
                  </div>
                  {modelsLoading ? (
                    <div className="flex items-center gap-2 text-[var(--color-label-tertiary)] text-[12px] py-1 mb-2">
                      <Loader2 className="w-3.5 h-3.5 animate-spin" />
                      {t("ui.loading", "Loading...")}
                    </div>
                  ) : modelsError ? (
                    <div role="alert" className="flex items-center justify-between gap-2 rounded-lg border border-red-500/20 bg-red-500/10 px-3 py-2 text-[12px] text-red-300 mb-2">
                      <span className="truncate">{modelsError}</span>
                      <button onClick={() => fetchCloudModels(providerId)} className="shrink-0 underline hover:no-underline">
                        {t("settings.models.retry", "Retry")}
                      </button>
                    </div>
                  ) : (
                    <div className="flex flex-wrap gap-1.5 mb-2">
                      {models.map((id) => {
                        const isActive = selectedModel === `${providerId}:${id}`;
                        return (
                          <button
                            key={id}
                            onClick={async () => {
                              try {
                                await modelSetDefault("chat", `${providerId}:${id}`);
                                setSelectedModel(`${providerId}:${id}`);
                                refresh();
                              } catch (e: any) {
                                console.error("set cloud narrator failed", e);
                              }
                            }}
                            className={cn(
                              "px-3 py-1.5 rounded-lg text-[12px] font-medium border transition-colors",
                              isActive
                                ? "border-[var(--color-accent)] bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
                                : "border-[var(--color-separator)] bg-black/10 text-[var(--color-label-secondary)] hover:border-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)]"
                            )}
                          >
                            {id}
                          </button>
                        );
                      })}
                      {models.length === 0 && (
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
                      const model = (manualModel[providerId] ?? "").trim();
                      if (model) void handleSetDefault(`${providerId}:${model}`);
                    }}
                  >
                    <input
                      value={manualModel[providerId] ?? ""}
                      onChange={(event) => setManualModel((m) => ({ ...m, [providerId]: event.target.value }))}
                      placeholder={t("ui.model-name", "Model name")}
                      aria-label={t("ui.custom-model-name", "Custom model name")}
                      className="min-w-0 flex-1 px-3 py-1.5 rounded-lg text-[12px] border border-[var(--color-separator)] bg-black/10 text-[var(--color-label-primary)] placeholder:text-[var(--color-label-tertiary)]"
                    />
                    <button
                      type="submit"
                      disabled={!(manualModel[providerId] ?? "").trim()}
                      className="px-3 py-1.5 rounded-lg text-[12px] font-medium border border-[var(--color-accent)] text-[var(--color-accent)] disabled:opacity-40"
                    >
                      {t("ui.use-model", "Use model")}
                    </button>
                  </form>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Hugging Face model browser — user can download any GGUF chat model */}
      <HfBrowser />

      {/* Active download progress */}
      {modelDownload?.phase === "active" && (
        <div className="p-4 rounded-xl bg-[var(--color-accent-soft)] border border-[var(--color-accent)]/20">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 text-sm text-[var(--color-accent)]">
              <Loader2 className="w-4 h-4 animate-spin" />
              Downloading {modelDownload.id}...
            </div>
            <button
              onClick={() => cancelDownload()}
              className="text-xs text-red-400 hover:text-red-300 transition-colors flex items-center gap-1"
            >
              <XCircle className="w-3 h-3" />
              {t("ui.cancel", "Cancel")}
            </button>
          </div>
          <div className="mt-2 text-xs text-[var(--color-label-tertiary)]">
            {modelDownload.status}
            {modelDownload.total > 0 && (
              <span> · {Math.round(modelDownload.completed / modelDownload.total * 100)}%</span>
            )}
          </div>
          <div className="mt-1.5 h-1 bg-[var(--color-fill-primary)] rounded-full overflow-hidden">
            <div
              className="h-full bg-[var(--color-accent)] transition-[width] duration-300"
              style={{
                width: modelDownload.total > 0
                  ? `${Math.min(100, Math.round(modelDownload.completed / modelDownload.total * 100))}%`
                  : "33%",
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}
