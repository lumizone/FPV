import { useEffect, useState } from "react";
import { Loader2, Cloud, Key, Trash2, Check, Save } from "lucide-react";
import { useTranslation } from "react-i18next";
import { byokSave, byokDelete, byokList, byokGetBaseUrl } from "@/lib/tauri";
import { toast } from "sonner";

/// Every provider the app can use for BYOK chat.
/// `logo` is the SVG filename in `public/icons/` — undefined = render
/// a generic Cloud icon (custom endpoint).
interface ProviderInfo {
  id: string;
  name: string;
  desc: string;
  placeholder: string;
  logo?: string;
}

const PROVIDERS: ProviderInfo[] = [
  { id: "openai", name: "OpenAI", desc: "GPT models — the standard for most.", placeholder: "sk-...", logo: "openai.svg" },
  { id: "anthropic", name: "Claude (Anthropic)", desc: "Claude Opus / Sonnet / Haiku.", placeholder: "sk-ant-...", logo: "anthropic.svg" },
  { id: "deepseek", name: "DeepSeek", desc: "Cheap, strong reasoning models.", placeholder: "sk-...", logo: "deepseek.svg" },
  { id: "google", name: "Google (Gemini)", desc: "Gemini Pro / Flash.", placeholder: "AIza...", logo: "google.svg" },
  { id: "openrouter", name: "OpenRouter", desc: "One key, many models.", placeholder: "sk-or-...", logo: "openrouter.svg" },
  { id: "mistral", name: "Mistral", desc: "Mistral Large / Small.", placeholder: "...", logo: "mistral.svg" },
  { id: "groq", name: "Groq", desc: "Very fast open models.", placeholder: "gsk_...", logo: "groq.svg" },
  { id: "xai", name: "xAI (Grok)", desc: "Grok models from xAI.", placeholder: "xai-...", logo: "xai.svg" },
  { id: "together", name: "Together.ai", desc: "200+ open models, one API.", placeholder: "..." },
  { id: "fireworks", name: "Fireworks.ai", desc: "Fast open-model inference.", placeholder: "fw_..." },
  { id: "novita", name: "Novita AI", desc: "Open models + GPU cloud.", placeholder: "..." },
  { id: "siliconflow", name: "SiliconFlow", desc: "Open models, China + global.", placeholder: "sk-..." },
  { id: "moonshot", name: "Moonshot AI (Kimi)", desc: "Kimi K-series models.", placeholder: "sk-..." },
  { id: "zhipu", name: "Zhipu (GLM)", desc: "GLM model family.", placeholder: "..." },
  { id: "qwen", name: "Qwen (Alibaba)", desc: "Qwen models via DashScope.", placeholder: "sk-..." },
  { id: "baichuan", name: "Baichuan", desc: "Baichuan model family.", placeholder: "sk-..." },
  { id: "minimax", name: "MiniMax", desc: "MiniMax model family.", placeholder: "..." },
  { id: "stepfun", name: "StepFun", desc: "Step model family.", placeholder: "..." },
  { id: "modelscope", name: "ModelScope", desc: "Alibaba's open model hub.", placeholder: "..." },
  { id: "xiaomimimo", name: "Xiaomi MiMo", desc: "Xiaomi's MiMo models.", placeholder: "..." },
  { id: "doubao", name: "Doubao (ByteDance)", desc: "Doubao models via Volcano Ark.", placeholder: "..." },
  { id: "hunyuan", name: "Tencent Hunyuan", desc: "Hunyuan T1 / Turbo models.", placeholder: "..." },
  { id: "upstage", name: "Upstage (Solar)", desc: "Solar Mini / Pro — Korea.", placeholder: "..." },
  { id: "yi", name: "01.AI (Yi)", desc: "Yi-Lightning — fast & cheap.", placeholder: "..." },
  { id: "plamo", name: "PLaMo (Preferred Networks)", desc: "PLaMo models — Japan.", placeholder: "..." },
  { id: "nvidia", name: "NVIDIA NIM", desc: "Llama, Mistral + more via one NVIDIA key.", placeholder: "nvapi-..." },
  { id: "cohere", name: "Cohere", desc: "Command R+ — strong multilingual.", placeholder: "..." },
  { id: "cerebras", name: "Cerebras", desc: "Ultra-fast wafer-scale inference.", placeholder: "..." },
  { id: "sambanova", name: "SambaNova", desc: "Very fast open-model inference.", placeholder: "..." },
  { id: "perplexity", name: "Perplexity", desc: "Sonar — online-aware models.", placeholder: "pplx-..." },
  { id: "ai21", name: "AI21 (Jamba)", desc: "Jamba hybrid models.", placeholder: "..." },
  { id: "venice", name: "Venice.ai", desc: "Private, uncensored models — text + image.", placeholder: "...", logo: "venice.svg" },
  { id: "bfl", name: "Black Forest Labs (FLUX)", desc: "FLUX image models — key for image generation.", placeholder: "..." },
  { id: "fal", name: "fal.ai", desc: "Image endpoints — you paste the endpoint id.", placeholder: "..." },
  { id: "custom", name: "Custom (OpenAI-compatible)", desc: "LM Studio, vLLM, SGLang, Ollama (remote), AIHubMix, 302.AI — any OpenAI-compatible endpoint. Local servers usually need no real key — any text will do.", placeholder: "sk-... (or any text for a local server)" },
];

/// Common OpenAI-compatible endpoints, pre-filled on click so users aren't
/// stuck hunting down a base URL by hand. Sources: vLLM/SGLang/Ollama/LM
/// Studio docs (local dev-server defaults) and AIHubMix/302.AI's own API docs
/// (hosted aggregators — many more models than this app lists by name).
const CUSTOM_BASE_URL_PRESETS: { label: string; url: string }[] = [
  { label: "LM Studio", url: "http://localhost:1234/v1" },
  { label: "Ollama", url: "http://localhost:11434/v1" },
  { label: "vLLM", url: "http://localhost:8000/v1" },
  { label: "SGLang", url: "http://localhost:30000/v1" },
  { label: "AIHubMix", url: "https://aihubmix.com/v1" },
  { label: "302.AI", url: "https://api.302.ai/v1" },
];

/// Settings tab for Bring-Your-Own-Key cloud providers. A clear list of
/// every supported provider — paste a key once, use it as the narrator
/// from Settings → Narrative Model, or for image generation.
export function ByokTab() {
  const { t } = useTranslation();
  const [configured, setConfigured] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(true);
  const [customBaseUrl, setCustomBaseUrl] = useState("");
  // Per-provider draft keys (not yet saved) + save state.
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState<Record<string, boolean>>({});
  const [saved, setSaved] = useState<Record<string, boolean>>({});

  const refresh = () => {
    setLoading(true);
    Promise.all([byokList(), byokGetBaseUrl()])
      .then(([res, baseUrl]) => {
        setConfigured(new Set(res.providers));
        if (baseUrl) setCustomBaseUrl(baseUrl);
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  };

  useEffect(() => { refresh(); }, []);

  const handleSave = async (p: ProviderInfo) => {
    const key = (draft[p.id] ?? "").trim();
    if (!key) return;
    setSaving((s) => ({ ...s, [p.id]: true }));
    try {
      await byokSave(p.id, key, p.id === "custom" ? customBaseUrl : undefined);
      setConfigured((prev) => new Set(prev).add(p.id));
      setDraft((d) => ({ ...d, [p.id]: "" }));
      setSaved((s) => ({ ...s, [p.id]: true }));
      setTimeout(() => setSaved((s) => ({ ...s, [p.id]: false })), 2000);
    } catch (e: any) {
      console.error("BYOK save failed", e);
      toast.error(t("settings.byok.save_failed", "Failed to save API key"));
    } finally {
      setSaving((s) => ({ ...s, [p.id]: false }));
    }
  };

  const handleDelete = async (p: ProviderInfo) => {
    if (!window.confirm(t("settings.byok.delete_confirm", "Remove {{name}} key?", { name: p.name }))) return;
    try {
      await byokDelete(p.id);
      setConfigured((prev) => {
        const next = new Set(prev);
        next.delete(p.id);
        return next;
      });
    } catch (e: any) {
      console.error("BYOK delete failed", e);
      toast.error(t("settings.byok.delete_failed", "Failed to remove API key"));
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-sm font-semibold text-[var(--color-label-primary)] mb-1">
          {t("settings.byok.title", "AI Cloud Connection")}
        </h3>
        <p className="text-[11px] text-[var(--color-label-tertiary)] mb-4">
          {t("settings.byok.desc", "Add your own API keys to use cloud LLMs as the narrator. FPV stays fully local by default — nothing is sent to any provider until you add a key here and pick a cloud model in Narrative Model.")}
        </p>
      </div>

      {loading ? (
        <div className="flex items-center gap-2 text-[var(--color-label-tertiary)] text-sm py-4">
          <Loader2 className="w-4 h-4 animate-spin" />
          {t("ui.loading", "Loading...")}
        </div>
      ) : (
        <div className="space-y-3">
          {PROVIDERS.map((p) => {
            const hasKey = configured.has(p.id);
            const savingHere = saving[p.id];
            const savedHere = saved[p.id];
            return (
              <div
                key={p.id}
                className="surface px-4 py-3"
              >
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-3 min-w-0">
                    {p.logo ? (
                      <img src={`/icons/${p.logo}`} alt="" className="w-5 h-5 shrink-0 rounded" />
                    ) : (
                      <Cloud className="w-5 h-5 text-[var(--color-label-secondary)] shrink-0" />
                    )}
                    <div className="min-w-0">
                      <div className="text-[14px] font-medium text-[var(--color-label-primary)]">
                        {p.name}
                      </div>
                      <div className="text-[12px] text-[var(--color-label-tertiary)] truncate">
                        {p.desc}
                      </div>
                    </div>
                  </div>
                  {hasKey ? (
                    <div className="flex items-center gap-2 shrink-0">
                      <span className="text-[11px] text-[var(--color-system-green)] flex items-center gap-1">
                        <Check className="w-3 h-3" />
                        {t("ui.key-saved", "Key saved")}
                      </span>
                      <button
                        onClick={() => handleDelete(p)}
                        className="p-1 text-[var(--color-label-tertiary)] hover:text-red-400 transition-colors"
                        title={t("ui.remove-key", "Remove key")}
                        aria-label={t("ui.remove-key-api", "Remove {{name}} API key", { name: p.name })}
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  ) : (
                    <span className="text-[11px] text-[var(--color-label-tertiary)] shrink-0">
                      {t("ui.no-key", "No key")}
                    </span>
                  )}
                </div>

                {/* Inline key entry */}
                <div className="mt-3 flex items-center gap-2">
                  <div className="relative flex-1">
                    <Key className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-[var(--color-label-tertiary)]" />
                    <input
                      type="password"
                      value={draft[p.id] ?? ""}
                      onChange={(e) =>
                        setDraft((d) => ({ ...d, [p.id]: e.target.value }))
                      }
                      placeholder={hasKey ? "Replace key…" : `Enter API key ${p.placeholder}`}
                      className="w-full pl-9 pr-3 py-1.5 text-[12px] rounded-lg bg-black/20 border border-white/10 text-[var(--color-label-primary)] placeholder:text-[var(--color-label-quaternary)] outline-none focus:border-[var(--color-accent)]/50"
                    />
                  </div>
                  {p.id === "custom" && (
                    <input
                      type="text"
                      value={customBaseUrl}
                      onChange={(e) => setCustomBaseUrl(e.target.value)}
                      placeholder="https://host:port/v1"
                      className="w-56 px-3 py-1.5 text-[12px] rounded-lg bg-black/20 border border-white/10 text-[var(--color-label-primary)] placeholder:text-[var(--color-label-quaternary)] outline-none focus:border-[var(--color-accent)]/50"
                    />
                  )}
                  <button
                    onClick={() => handleSave(p)}
                    disabled={savingHere || !(draft[p.id] ?? "").trim()}
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-[var(--color-accent)] text-black text-[12px] font-semibold hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] disabled:opacity-40 transition-colors shrink-0"
                  >
                    {savingHere ? (
                      <Loader2 className="w-3.5 h-3.5 animate-spin" />
                    ) : savedHere ? (
                      <Check className="w-3.5 h-3.5" />
                    ) : (
                      <Save className="w-3.5 h-3.5" />
                    )}
                    {savedHere ? "Saved" : "Save"}
                  </button>
                </div>
                {p.id === "custom" && (
                  <div className="mt-2 flex flex-wrap items-center gap-1.5">
                    <span className="text-[11px] text-[var(--color-label-tertiary)] mr-0.5">
                      {t("ui.quick-fill", "Quick fill:")}
                    </span>
                    {CUSTOM_BASE_URL_PRESETS.map((preset) => (
                      <button
                        key={preset.label}
                        type="button"
                        onClick={() => setCustomBaseUrl(preset.url)}
                        className="px-2 py-1 rounded-md text-[11px] font-medium border border-[var(--color-separator)] text-[var(--color-label-secondary)] hover:border-[var(--color-accent)]/50 hover:text-[var(--color-label-primary)] transition-colors"
                      >
                        {preset.label}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
