import { useEffect, useRef, useState } from "react";
import {
  Search,
  Download,
  ChevronDown,
  Loader2,
  ExternalLink,
  HardDrive,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { hfSearch, hfQuants, type HfModel, type HfQuant } from "@/lib/tauri";
import { useApp } from "@/lib/store";
import { cn } from "@/lib/utils";

/// Parse a pasted Hugging Face URL or `owner/name` into a repo id.
function parseHfRepo(input: string): string | null {
  const s = input.trim();
  const url = s.match(/huggingface\.co\/([^/\s?#]+)\/([^/\s?#]+)/i);
  if (url) return `${url[1]}/${url[2]}`;
  if (/^[\w.-]+\/[\w.-]+$/.test(s)) return s;
  return null;
}

/// "Browse Hugging Face" — search GGUF chat models on HF and install
/// them through the bundled Ollama (`hf.co/{repo}:{quant}`).
export function HfBrowser() {
  const { t } = useTranslation();
  const startDownload = useApp((s) => s.startModelDownload);
  const modelDownload = useApp((s) => s.model_download);

  const [query, setQuery] = useState("");
  const [results, setResults] = useState<HfModel[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [quants, setQuants] = useState<HfQuant[] | null>(null);
  const [quantsLoading, setQuantsLoading] = useState(false);
  const quantReq = useRef(0);

  // Debounced search.
  useEffect(() => {
    const q = query.trim();
    if (q.length < 2) {
      setResults(null);
      setSearching(false);
      return;
    }
    const repo = parseHfRepo(q);
    if (repo) {
      // Pasted a repo → treat as a single result and open its quants.
      setSearching(false);
      setError(null);
      setResults([{ id: repo, downloads: 0, likes: 0, tags: [] }]);
      if (expanded !== repo) void openQuants(repo);
      return;
    }
    let cancelled = false;
    setSearching(true);
    setError(null);
    const handle = setTimeout(() => {
      hfSearch(q)
        .then((res) => {
          if (!cancelled) setResults(res);
        })
        .catch((e: any) => {
          if (!cancelled) setError(e?.message ?? String(e));
        })
        .finally(() => {
          if (!cancelled) setSearching(false);
        });
    }, 350);
    return () => {
      cancelled = true;
      clearTimeout(handle);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  async function openQuants(repo: string) {
    if (expanded === repo) {
      setExpanded(null);
      return;
    }
    setExpanded(repo);
    setQuants(null);
    setQuantsLoading(true);
    const reqId = ++quantReq.current;
    try {
      const q = await hfQuants(repo);
      if (quantReq.current === reqId) setQuants(q);
    } catch (e: any) {
      if (quantReq.current === reqId) setQuants([]);
    } finally {
      if (quantReq.current === reqId) setQuantsLoading(false);
    }
  }

  function fmtSize(b: number): string {
    if (!b) return "?";
    const gb = b / 1_000_000_000;
    return gb >= 1 ? `${gb.toFixed(1)} GB` : `${(b / 1_000_000).toFixed(0)} MB`;
  }

  function install(repo: string, quant: string) {
    // Ollama pulls HF GGUF via `hf.co/{repo}:{quant}`.
    startDownload(`hf.co/${repo}:${quant}`, "chat");
  }

  return (
    <div className="surface px-4 py-4 space-y-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <HardDrive className="w-4 h-4 text-[var(--color-label-secondary)]" />
          <span className="text-[13px] font-medium text-[var(--color-label-primary)]">
            {t("settings.models.hf_title", "Browse Hugging Face")}
          </span>
        </div>
        <button
          type="button"
          onClick={() => void openUrl("https://huggingface.co/models?library=gguf")}
          className="text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] inline-flex items-center gap-1"
          title={t("ui.open-huggingface-co", "Open huggingface.co")}
        >
          <ExternalLink className="w-3 h-3" />
          huggingface.co
        </button>
      </div>

      <p className="text-[11px] text-[var(--color-label-tertiary)]">
        {t(
          "settings.models.hf_hint",
          "Search GGUF chat models and install them as the narrator. Any quant that runs in Ollama works."
        )}
      </p>

      <div className="relative">
        <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-[var(--color-label-tertiary)]" />
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("settings.models.hf_placeholder", "Search or paste a repo URL (e.g. bartowski/Llama-3.1-8B)…")}
          className="w-full pl-9 pr-3 py-1.5 text-[12px] rounded-lg bg-black/20 border border-white/10 text-[var(--color-label-primary)] placeholder:text-[var(--color-label-quaternary)] outline-none focus:border-[var(--color-accent)]/50"
        />
      </div>

      {error && <p className="text-[11px] text-red-400">{error}</p>}

      {searching && (
        <div className="flex items-center gap-2 text-[11px] text-[var(--color-label-tertiary)]">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          {t("ui.searching", "Searching…")}
        </div>
      )}

      {results && results.length === 0 && !searching && (
        <p className="text-[11px] text-[var(--color-label-tertiary)]">
          {t("ui.no-gguf-models-found", "No GGUF models found.")}
        </p>
      )}

      <div className="space-y-1.5">
        {results?.map((m) => {
          const isOpen = expanded === m.id;
          return (
            <div key={m.id} className="border border-white/5 rounded-lg">
              <button
                onClick={() => openQuants(m.id)}
                className="w-full flex items-center justify-between px-3 py-2 text-left hover:bg-black/10 rounded-lg transition-colors"
              >
                <div className="min-w-0">
                  <div className="text-[12px] font-medium text-[var(--color-label-primary)] truncate">
                    {m.id}
                  </div>
                  <div className="text-[10px] text-[var(--color-label-tertiary)]">
                    {m.downloads > 0 ? `${m.downloads.toLocaleString()} downloads` : ""}
                    {m.likes > 0 ? ` · ${m.likes} likes` : ""}
                  </div>
                </div>
                <ChevronDown
                  className={cn(
                    "w-4 h-4 text-[var(--color-label-tertiary)] shrink-0 transition-transform",
                    isOpen && "rotate-180"
                  )}
                />
              </button>

              {isOpen && (
                <div className="px-3 pb-3 space-y-1.5">
                  {quantsLoading && (
                    <div className="flex items-center gap-2 text-[11px] text-[var(--color-label-tertiary)]">
                      <Loader2 className="w-3.5 h-3.5 animate-spin" />
                      {t("ui.loading-quants", "Loading quants…")}
                    </div>
                  )}
                  {!quantsLoading && quants && quants.length === 0 && (
                    <p className="text-[11px] text-[var(--color-label-tertiary)]">
                      {t("ui.no-gguf-files-in-this-repo", "No GGUF files in this repo.")}
                    </p>
                  )}
                  {quants?.slice(0, 8).map((q) => {
                    const isDownloading =
                      modelDownload?.phase === "active" &&
                      modelDownload.id === `hf.co/${m.id}:${q.quant}`;
                    return (
                      <div
                        key={q.filename}
                        className="flex items-center justify-between py-1.5 px-2 rounded-lg bg-black/10"
                      >
                        <div className="min-w-0">
                          <div className="text-[11px] font-medium text-[var(--color-label-primary)] truncate">
                            {q.quant || q.filename}
                          </div>
                          <div className="text-[10px] text-[var(--color-label-tertiary)] truncate">
                            {q.filename} · {fmtSize(q.size)}
                          </div>
                        </div>
                        <button
                          onClick={() => install(m.id, q.quant)}
                          disabled={modelDownload?.phase === "active"}
                          className="inline-flex items-center gap-1 px-2.5 py-1 rounded-md bg-[var(--color-accent)] text-black text-[10px] font-semibold hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] disabled:opacity-40 transition-colors shrink-0"
                        >
                          {isDownloading ? (
                            <Loader2 className="w-3 h-3 animate-spin" />
                          ) : (
                            <Download className="w-3 h-3" />
                          )}
                          {isDownloading ? "…" : "Install"}
                        </button>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
