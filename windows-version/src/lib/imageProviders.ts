import { type ImageProviderId } from "@/lib/tauri";

/// Jedna karta chmurowa w pickerze providerów obrazu.
/// id i byokKey muszą zgadzać się z backendem: ImageProvider::as_str()
/// i byok-provider id (wpięte w Taskach 1 i 4 projektu fal).
export type ImageProviderEntry = {
  id: ImageProviderId; // wire id, np. "flux"
  byokKey: string;     // BYOK text-provider id trzymający klucz, np. "bfl"
  labelKey: string;    // klucz i18n nazwy (fallback = en)
  labelFallback: string;
  descKey: string;
  descFallback: string;
  setupKey: string;    // "Dodaj klucz X w BYOK" — gdy klucza nie ma
  setupFallback: string;
};

export const CLOUD_IMAGE_PROVIDERS: readonly ImageProviderEntry[] = [
  // Kolejność = kolejność kart w UI (po karcie local). Świadomie jawna.
  { id: "openai",  byokKey: "openai", labelKey: "settings.image.provider_openai",       labelFallback: "OpenAI Image API",                        descKey: "settings.image.provider_openai_desc",       descFallback: "Uses your saved OpenAI API key.",        setupKey: "settings.image.provider_openai_setup",       setupFallback: "Add an OpenAI key in BYOK" },
  { id: "seedream", byokKey: "doubao", labelKey: "settings.image.provider_seedream",     labelFallback: "Seedream (ByteDance)",                   descKey: "settings.image.provider_seedream_desc",     descFallback: "Uses your Doubao (Volcano Ark) API key.", setupKey: "settings.image.provider_seedream_setup",     setupFallback: "Add a Doubao key in BYOK" },
  { id: "hunyuan", byokKey: "hunyuan", labelKey: "settings.image.provider_hunyuan",      labelFallback: "Hunyuan-Image (Tencent)",                descKey: "settings.image.provider_hunyuan_desc",      descFallback: "Uses your Hunyuan API key.",              setupKey: "settings.image.provider_hunyuan_setup",      setupFallback: "Add a Hunyuan key in BYOK" },
  { id: "cogview", byokKey: "zhipu",   labelKey: "settings.image.provider_cogview",      labelFallback: "CogView (Zhipu)",                        descKey: "settings.image.provider_cogview_desc",      descFallback: "Uses your Zhipu API key.",                setupKey: "settings.image.provider_cogview_setup",      setupFallback: "Add a Zhipu key in BYOK" },
  { id: "flux",    byokKey: "bfl",     labelKey: "settings.image.provider_flux",         labelFallback: "FLUX (Black Forest Labs)",               descKey: "settings.image.provider_flux_desc",         descFallback: "Uses your BFL API key.",                  setupKey: "settings.image.provider_flux_setup",         setupFallback: "Add a BFL key in BYOK" },
  { id: "fal",     byokKey: "fal",     labelKey: "ui.fal-ai",                            labelFallback: "fal.ai",                                descKey: "ui.fal-desc",                                descFallback: "Paste any fal.ai image endpoint id — the endpoint is the model.", setupKey: "settings.image.provider_fal_setup", setupFallback: "Add a fal.ai key in BYOK" },
  { id: "imagen",  byokKey: "google",  labelKey: "settings.image.provider_imagen",       labelFallback: "Google Imagen (Gemini)",                 descKey: "settings.image.provider_imagen_desc",       descFallback: "Uses your Google API key.",               setupKey: "settings.image.provider_imagen_setup",       setupFallback: "Add a Google key in BYOK" },
];

/// Wspólna linia statusu dla kart chmurowych: klucz podpięty / brak klucza.
/// Jeden klucz zamiast siedmiu kopii "API key connected".
export const CLOUD_PROVIDER_READY_KEY = "settings.image.provider_ready";
export const CLOUD_PROVIDER_READY_FALLBACK = "API key connected";
