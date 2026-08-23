//! macOS local image backend: stable-diffusion.cpp (`sd-cli`).
//!
//! `ImageProvider::Local` runs the same backend the Windows fork shipped
//! in v1.5.0: a self-contained native binary — the architectural twin of
//! the bundled Ollama runtime. The binary is BUNDLED in `Contents/MacOS`
//! and signed by the packaging script (`scripts/package.sh` for the
//! direct build); only the model WEIGHTS download at runtime, which is
//! data, not code — same category as Ollama models. This replaced the old
//! mflux flow because it is smaller, faster, and uses less RAM.
//!
//! Ported from the Windows fork's `image/sdcpp.rs` (2026-07-09,
//! real-hardware verified 2026-07-15). The pure parts — manifest,
//! CLI-arg builder, progress parser — are kept byte-compatible with the
//! win original so fixes merge across forks cleanly. Deliberate
//! differences:
//!   - `sd_cli_path` resolves next to `current_exe()` (the bundled copy)
//!     instead of a resource dir, and there is NO `fetch_gpu_runtime` —
//!     Metal is compiled into the binary (`cmake -DSD_METAL=ON`), there
//!     is nothing faster to fetch, and fetching a binary would be 2.5.2
//!     anyway.
//!   - `download_model` reports through a `FnMut(u8, &str)` callback
//!     (the shape `image::local::prewarm` already plumbs to the
//!     `image:prewarm` Tauri event) instead of taking an `AppHandle`.
//!   - no `CREATE_NO_WINDOW` (a Windows console concept).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{AppError, AppResult};

const MAX_RENDERED_PNG_BYTES: usize = 32 * 1024 * 1024;

fn validate_png(bytes: &[u8]) -> AppResult<()> {
    crate::image::validate_png_payload(bytes, MAX_RENDERED_PNG_BYTES)
        .map_err(|error| AppError::Other(error.to_string()))
}

/// Live progress (0-100) of the CURRENT local render, published for the
/// UI's "sending a photo…" chip. A single global slot is correct because
/// local renders are serialized (`LOCAL_RENDER_LOCK` below); cloud
/// providers never publish, so the chip falls back to an indeterminate
/// bar there.
fn render_progress_tx() -> &'static tokio::sync::watch::Sender<u8> {
    static TX: std::sync::OnceLock<tokio::sync::watch::Sender<u8>> = std::sync::OnceLock::new();
    TX.get_or_init(|| tokio::sync::watch::channel(0).0)
}

/// Subscribe to the current render's progress (percent 0-100).
/// NOTE: publish via `send_replace`, never `send` — `send` silently
/// drops the value when no receiver is subscribed (the Telegram render
/// path has no UI forwarder).
pub fn subscribe_render_progress() -> tokio::sync::watch::Receiver<u8> {
    render_progress_tx().subscribe()
}

/// Parse one stable-diffusion.cpp output frame into a percent.
///
/// sd.cpp redraws its sampling bar with bare `\r` frames shaped like:
///   `  |==================>               | 12/20 - 4.50it/s`
/// Tolerant matcher: any `cur/total` integer token on a frame that looks
/// like the progress bar (contains `|` or an it/s rate). Log lines
/// return `None`, so misparses never move the bar.
pub fn parse_progress_line(line: &str) -> Option<u8> {
    if !(line.contains('|') || line.contains("it/s") || line.contains("s/it")) {
        return None;
    }
    for tok in line.split_whitespace() {
        if let Some((a, b)) = tok.split_once('/') {
            if let (Ok(cur), Ok(total)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                if total > 0 && cur <= total {
                    return Some(((cur * 100) / total).min(100) as u8);
                }
            }
        }
    }
    None
}

/// Best-effort removal of the decoded portrait/kontext reference PNGs.
/// Shared by every exit path out of the render call (success, a wait()
/// error, and a timeout) so none of them can leak these into the temp
/// dir — a bare early `?` used to skip this entirely on the error path.
async fn cleanup_temp_files(
    init_tmp: Option<std::path::PathBuf>,
    ref_tmps: Vec<std::path::PathBuf>,
) {
    if let Some(p) = init_tmp {
        let _ = tokio::fs::remove_file(p).await;
    }
    for p in ref_tmps {
        let _ = tokio::fs::remove_file(p).await;
    }
}

/// Stream a child pipe through `parse_progress_line`, publishing hits to
/// the render-progress channel. Splits on BOTH `\n` and `\r` — sd.cpp
/// redraws its bar with bare carriage returns.
async fn pump_progress<R: tokio::io::AsyncRead + Unpin>(pipe: R) {
    use tokio::io::AsyncReadExt;
    let mut reader = tokio::io::BufReader::new(pipe);
    let mut buf = [0u8; 4096];
    let mut line = String::new();
    loop {
        let n = reader.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            if b == b'\n' || b == b'\r' {
                if let Some(pct) = parse_progress_line(&line) {
                    render_progress_tx().send_replace(pct);
                }
                line.clear();
            } else if line.len() < 512 {
                line.push(b as char);
            }
        }
    }
}

/// One downloadable weight file the local model needs — these models are
/// multi-file: a diffusion model, a VAE, and a text encoder (FLUX.2 uses a
/// single LLM encoder; FLUX.1 used CLIP-L + T5XXL).
#[derive(Debug, Clone, Copy)]
pub struct ManifestFile {
    /// Role → the sd-cli flag it maps to, verbatim (`build_sd_args` emits
    /// `--{role}`): `diffusion-model` | `vae` | `llm` | `clip_l` | `t5xxl`
    /// | `model` (SDXL single-file load, `-m`/`--model`).
    pub role: &'static str,
    /// Filename on disk (also the value passed to the sd-cli path flag).
    pub filename: &'static str,
    /// Direct download URL. Sizes + reachability verified by HTTP
    /// content-length from this host on 2026-07-16.
    pub url: &'static str,
    /// Approx download size in MiB (progress weighting + disk guard).
    pub size_mib: u64,
    /// SHA-256 of the immutable upstream file. Empty is reserved for local
    /// test fixtures only; production manifests must pin a digest.
    pub sha256: &'static str,
}

/// Default model: **FLUX.2-klein-4B** (4-step distilled).
///
/// Chosen over FLUX.1-schnell on 2026-07-16 after rendering the SAME
/// portrait + prompt + params through both on an M3 Pro. klein won on
/// every axis that matters:
///
/// | | FLUX.1-schnell | FLUX.2-klein-4B |
/// |---|---|---|
/// | download | 17.6 GB | **4.8 GB** |
/// | render | 73 s | **29.7 s** |
/// | selfie pose ("arm extended holding the phone") | ignored | **honoured** |
/// | hands | deformed | clean |
/// | licence | Apache, but the VAE came from an unofficial ungated mirror | **Apache, every file straight from black-forest-labs** |
///
/// The prompt-following gain is not luck: klein encodes text with
/// **Qwen3-4B** instead of T5XXL+CLIP-L, and it shows on natural-language
/// scene descriptions — which is exactly what `selfie::build_prompt`
/// emits. It also drops the FLUX.1 licence wart: schnell's VAE is gated
/// at BFL, so the win fork had to pull it from `camenduru/…-ungated`;
/// every klein file here is ungated and Apache-2.0 at source.
///
/// Sizes verified by HTTP content-length 2026-07-16; all four render
/// params (`--cfg-scale 1.0 --steps 4`) match sd.cpp's own `docs/flux2.md`
/// for this checkpoint.
pub const FLUX2_KLEIN_4B: &[ManifestFile] = &[
    ManifestFile {
        role: "diffusion-model",
        filename: "flux-2-klein-4b-Q4_0.gguf",
        url: "https://huggingface.co/leejet/FLUX.2-klein-4B-GGUF/resolve/3b1f5a9dc3abb32238b053aeb3d823c30afdacbd/flux-2-klein-4b-Q4_0.gguf",
        size_mib: 2345,
        sha256: "d1023499ef3f2f82ff7c50e6778495195c1b6cc34835741778868428111f9ff4",
    },
    ManifestFile {
        // FLUX.2 replaces T5XXL+CLIP-L with a single LLM text encoder.
        role: "llm",
        filename: "Qwen3-4B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-GGUF/resolve/22c9fc8a8c7700b76a1789366280a6a5a1ad1120/Qwen3-4B-Q4_K_M.gguf",
        size_mib: 2386,
        sha256: "f6f851777709861056efcdad3af01da38b31223a3ba26e61a4f8bf3a2195813a",
    },
    ManifestFile {
        // From the klein-4B repo itself (Apache-2.0, ungated) — NOT the
        // gated FLUX.2-dev repo sd.cpp's docs happen to point at.
        role: "vae",
        filename: "flux2_ae.safetensors",
        url: "https://huggingface.co/black-forest-labs/FLUX.2-klein-4B/resolve/e7b7dc27f91deacad38e78976d1f2b499d76a294/vae/diffusion_pytorch_model.safetensors",
        size_mib: 164,
        sha256: "ca70d2202afe6415bdbcb8793ba8cd99fd159cfe6192381504d6c4d3036e0f04",
    },
];

/// **Z-Image-Turbo** — the same checkpoint mflux runs in the direct
/// build, so picking it gives cross-edition parity on renders the user
/// already knows from production.
///
/// Measured against klein on the identical portrait/prompt/params
/// (2026-07-16): nicer face, but it lost the portrait's outfit and the
/// selfie pose, at 6.1 GB / 44 s vs klein's 4.8 GB / 29.7 s. Offered as a
/// choice rather than the default for exactly that reason — the face is a
/// matter of taste, the pose and outfit are not.
///
/// Shares klein's **Qwen3-4B** encoder, so switching costs only this
/// model's own diffusion file. Its VAE is FLUX.1's (per sd.cpp's
/// `docs/z_image.md`) — a different file from klein's FLUX.2 VAE, hence
/// its own manifest entry. Sizes verified by content-length 2026-07-16.
pub const Z_IMAGE_TURBO: &[ManifestFile] = &[
    ManifestFile {
        role: "diffusion-model",
        filename: "z_image_turbo-Q4_0.gguf",
        url: "https://huggingface.co/leejet/Z-Image-Turbo-GGUF/resolve/c61c0e422dc8b541b7548cf33a4ef8302b0f8085/z_image_turbo-Q4_0.gguf",
        size_mib: 3512,
        sha256: "2bc57986874c84f7ec6d02d9d7070a53b0029954a0e38a6e1342eb91095572f5",
    },
    ManifestFile {
        role: "llm",
        filename: "Qwen3-4B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-GGUF/resolve/22c9fc8a8c7700b76a1789366280a6a5a1ad1120/Qwen3-4B-Q4_K_M.gguf",
        size_mib: 2386,
        sha256: "f6f851777709861056efcdad3af01da38b31223a3ba26e61a4f8bf3a2195813a",
    },
    ManifestFile {
        // Z-Image uses the FLUX.1 VAE (sd.cpp docs/z_image.md). The
        // official BFL FLUX.1-schnell repo is gated, so this is the same
        // ungated mirror the win fork settled on.
        role: "vae",
        filename: "flux1_ae.safetensors",
        url: "https://huggingface.co/camenduru/FLUX.1-dev-ungated/resolve/83135205023a8c61cc8190b99abc82332e27c778/ae.safetensors",
        size_mib: 335,
        sha256: "afc8e28272cd15db3919bacdb6918ce9c1ed22e96cb12c4d5ed0fba823529e38",
    },
];

/// **FLUX.2-klein-base-4B** — the NON-distilled sibling of the default.
/// Same weights family, but trained for a full sampling schedule
/// (`--cfg-scale 4.0`, ~20 steps per sd.cpp's docs/flux2.md) instead of
/// klein's 4-step distillation — slower, with headroom for more detail.
/// Shares BOTH the Qwen3-4B encoder and the FLUX.2 VAE with the default,
/// so it costs only its own 2.29 GB diffusion file.
pub const FLUX2_KLEIN_BASE_4B: &[ManifestFile] = &[
    ManifestFile {
        role: "diffusion-model",
        filename: "flux-2-klein-base-4b-Q4_0.gguf",
        url: "https://huggingface.co/leejet/FLUX.2-klein-base-4B-GGUF/resolve/d12671125306ca6b5f6db1b33ed4c80c8511a53f/flux-2-klein-base-4b-Q4_0.gguf",
        size_mib: 2345,
        sha256: "c3a2854510677b7aa37dd7547d908c54889a76c6d6aa3ffe902fcaa092d1328b",
    },
    ManifestFile {
        role: "llm",
        filename: "Qwen3-4B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-GGUF/resolve/22c9fc8a8c7700b76a1789366280a6a5a1ad1120/Qwen3-4B-Q4_K_M.gguf",
        size_mib: 2386,
        sha256: "f6f851777709861056efcdad3af01da38b31223a3ba26e61a4f8bf3a2195813a",
    },
    ManifestFile {
        role: "vae",
        filename: "flux2_ae.safetensors",
        url: "https://huggingface.co/black-forest-labs/FLUX.2-klein-4B/resolve/e7b7dc27f91deacad38e78976d1f2b499d76a294/vae/diffusion_pytorch_model.safetensors",
        size_mib: 164,
        sha256: "ca70d2202afe6415bdbcb8793ba8cd99fd159cfe6192381504d6c4d3036e0f04",
    },
];

/// **Animagine XL 4.0** — anime-NATIVE SDXL fine-tune (cagliostrolab).
/// The pool's first style model: the FLUX/Z-Image trio are general-purpose
/// checkpoints that lean photoreal, and "anime" was only ever a prompt
/// hint. SDXL loads as a SINGLE file (CLIP-L/G + VAE baked in) via
/// `--model`; the standalone fp16-fix VAE rides along because SDXL's own
/// fp16 VAE NaNs (sd.cpp docs/sd.md). OpenRAIL++-M, ungated, sizes
/// verified by content-length 2026-07-17. Measured that day (M3 Pro):
/// ~199 s/render, peak 11.2 GiB (→ 16 GiB `min_ram_gib`). Anime-native,
/// excellent for English prompts; danbooru "1girl" bias; the CLIP encoder
/// does NOT understand non-English prompts (a Polish prompt rendered the
/// wrong scene — use klein for those).
pub const ANIMAGINE_XL_40: &[ManifestFile] = &[
    ManifestFile {
        role: "model",
        filename: "animagine-xl-4.0-opt.safetensors",
        url: "https://huggingface.co/cagliostrolab/animagine-xl-4.0/resolve/2b7c1b397761bf5bd3cc42e5b39ec99314a75a96/animagine-xl-4.0-opt.safetensors",
        size_mib: 6616,
        sha256: "6327eca98bfb6538dd7a4edce22484a1bbc57a8cff6b11d075d40da1afb847ac",
    },
    ManifestFile {
        // madebyollin's fp16-fix (MIT, ungated) — the standard cure for
        // SDXL's fp16 VAE NaN. SHARED with RealVis (same filename ⇒ same
        // URL invariant), so the second SDXL model skips these 319 MiB.
        role: "vae",
        filename: "sdxl_vae_fp16_fix.safetensors",
        url: "https://huggingface.co/madebyollin/sdxl-vae-fp16-fix/resolve/207b116dae70ace3637169f1ddd2434b91b3a8cd/sdxl_vae.safetensors",
        size_mib: 319,
        sha256: "235745af8d86bf4a4c1b5b4f529868b37019a10f7c0b2e79ad0abca3a22bc6e1",
    },
];

/// **RealVisXL V5.0** — photorealism-focused SDXL fine-tune (SG161222).
/// Covers the "premium photoreal" gap for users whose character portrait
/// is photographic. openrail++, ungated, single-file + the same shared
/// fp16-fix VAE as Animagine. Sizes verified 2026-07-17. Measured that day
/// (M3 Pro): ~212 s/render, peak 11.2 GiB (→ 16 GiB `min_ram_gib`); cfg
/// 6.0 confirmed by A/B (5/6/7 all good, 6 the middle). Excellent for
/// English prompts; like Animagine its CLIP encoder does NOT understand
/// non-English prompts.
pub const REALVIS_XL_50: &[ManifestFile] = &[
    ManifestFile {
        role: "model",
        filename: "RealVisXL_V5.0_fp16.safetensors",
        url: "https://huggingface.co/SG161222/RealVisXL_V5.0/resolve/ac93e0dda1f6d448cae19bbfab8c5e720a5e48bc/RealVisXL_V5.0_fp16.safetensors",
        size_mib: 6616,
        sha256: "6a35a7855770ae9820a3c931d4964c3817b6d9e3c6f9c4dabb5b3a94e5643b80",
    },
    ManifestFile {
        role: "vae",
        filename: "sdxl_vae_fp16_fix.safetensors",
        url: "https://huggingface.co/madebyollin/sdxl-vae-fp16-fix/resolve/207b116dae70ace3637169f1ddd2434b91b3a8cd/sdxl_vae.safetensors",
        size_mib: 319,
        sha256: "235745af8d86bf4a4c1b5b4f529868b37019a10f7c0b2e79ad0abca3a22bc6e1",
    },
];

// **Z-Image (non-turbo) was evaluated and REJECTED on 2026-07-16.** With
// this sd.cpp build (master a8a91b2) and unsloth's z-image-Q4_0.gguf, it
// produced a BLANK WHITE image in both img2img (cfg 5.0, 248 s) and plain
// txt2img (346 s) on an M3 Pro — the doc-recommended params from
// docs/z_image.md, the same encoder + VAE combination the working models
// use. Whatever the root cause (quant, VAE latent format, prediction
// mode), it does not render, so it is not offered. Re-evaluate against a
// newer sd.cpp / different quant before ever re-adding.

/// Which local checkpoint to render with. Persisted as
/// `app_meta["image_local_model"]`.
///
/// Deliberately NOT offered: **FLUX.2-klein-9B**. Its GGUF repo (leejet)
/// declares no licence, and the upstream source
/// `black-forest-labs/FLUX.2-klein-9B` is `license: other` + `gated: auto`
/// — not Apache, and it needs an HF token. Verified 2026-07-16; don't add
/// it on the strength of klein-4B being Apache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocalModel {
    /// FLUX.2-klein-4B — default. Smallest, fastest, best prompt-following.
    #[default]
    Klein4B,
    /// Z-Image-Turbo — parity with the direct build's mflux checkpoint.
    ZImageTurbo,
    /// FLUX.2-klein-base-4B — LARGE: non-distilled, full schedule.
    KleinBase4B,
    /// Animagine XL 4.0 — anime-native SDXL (danbooru-style training).
    Animagine40,
    /// RealVisXL V5.0 — photorealistic SDXL.
    RealVisXl50,
}

impl LocalModel {
    pub const CHOICES: [LocalModel; 5] = [
        LocalModel::Klein4B,
        LocalModel::ZImageTurbo,
        LocalModel::KleinBase4B,
        LocalModel::Animagine40,
        LocalModel::RealVisXl50,
    ];

    /// The "large" tag the Settings UI shows: non-distilled models that
    /// run a full sampling schedule — noticeably slower, offered for
    /// people who trade time for detail. The two SDXL models join this
    /// class until they're measured on hardware (full sampling schedule,
    /// no distillation — same reasoning as KleinBase4B).
    pub fn is_large(self) -> bool {
        matches!(
            self,
            Self::KleinBase4B | Self::Animagine40 | Self::RealVisXl50
        )
    }

    /// Minimum system RAM (GiB) to offer this checkpoint. The FLUX/Z-Image
    /// family keeps the pool's 12 GiB floor (klein peaked at 8.05 GiB,
    /// measured 2026-07-16). The SDXL models are unquantised fp16 (6.6 GB
    /// weights) rendering at 832×1216 and peaked at 11.2 GiB (measured
    /// 2026-07-17) — they need 16 GiB, keeping klein's ~1.5× peak-to-floor
    /// headroom so a resident chat model doesn't push a render into swap.
    pub fn min_ram_gib(self) -> u64 {
        match self {
            Self::Klein4B | Self::ZImageTurbo | Self::KleinBase4B => {
                crate::image::local_min_ram_gb()
            }
            Self::Animagine40 | Self::RealVisXl50 => 16,
        }
    }

    /// CFG scale per each checkpoint's model card. Distilled FLUX/Z-Image
    /// models are trained for guidance 1.0; the base FLUX model and both
    /// SDXL fine-tunes need real CFG. RealVisXL's card gives no number;
    /// 6.0 was confirmed by an A/B at the 2026-07-17 measurement session
    /// (5/6/7 all rendered well on a fixed seed, 6 the middle ground).
    pub fn cfg_scale(self) -> f32 {
        match self {
            Self::Klein4B | Self::ZImageTurbo => 1.0,
            Self::KleinBase4B => 4.0,
            Self::Animagine40 => 5.0,
            Self::RealVisXl50 => 6.0,
        }
    }

    /// Doc-recommended default steps: klein-4b 4, z-image-turbo 8
    /// (docs/z_image.md — the win fork's mflux cap agrees), base FLUX 20.
    /// Animagine 28 and RealVis 30 come from their own model cards.
    pub fn default_steps(self) -> u32 {
        match self {
            Self::Klein4B => 4,
            Self::ZImageTurbo => 8,
            Self::KleinBase4B => 20,
            Self::Animagine40 => 28,
            Self::RealVisXl50 => 30,
        }
    }

    /// The step choices the Settings control offers for THIS model. A
    /// distilled model's menu starts at its sweet spot; a base model's
    /// brackets its recommended default. Per-model rather than a
    /// `is_large()` branch now that the two SDXL cards each specify their
    /// own bracket.
    pub fn step_choices(self) -> [u32; 4] {
        match self {
            Self::Klein4B | Self::ZImageTurbo => [4, 8, 12, 16],
            Self::KleinBase4B => [12, 20, 28, 36],
            Self::Animagine40 => [20, 24, 28, 32],
            Self::RealVisXl50 => [24, 30, 40, 50],
        }
    }

    /// Steps actually passed to sd-cli: the stored/requested value if this
    /// model offers it, else the model's own default — a stored choice
    /// from one model must not leak into another (4 steps on a base model
    /// would produce noise).
    pub fn sanitize_steps(self, v: u32) -> u32 {
        if self.step_choices().contains(&v) {
            v
        } else {
            self.default_steps()
        }
    }

    /// Stable id for `app_meta` + the frontend. Changing these strings
    /// orphans a user's stored choice (it falls back to the default) and
    /// their downloaded weights (a fresh dir) — so don't.
    pub fn id(self) -> &'static str {
        match self {
            Self::Klein4B => "flux2-klein-4b",
            Self::ZImageTurbo => "z-image-turbo",
            Self::KleinBase4B => "flux2-klein-base-4b",
            Self::Animagine40 => "animagine-xl-4",
            Self::RealVisXl50 => "realvis-xl-5",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Self::CHOICES.into_iter().find(|m| m.id() == s)
    }

    pub fn manifest(self) -> &'static [ManifestFile] {
        match self {
            Self::Klein4B => FLUX2_KLEIN_4B,
            Self::ZImageTurbo => Z_IMAGE_TURBO,
            Self::KleinBase4B => FLUX2_KLEIN_BASE_4B,
            Self::Animagine40 => ANIMAGINE_XL_40,
            Self::RealVisXl50 => REALVIS_XL_50,
        }
    }

    /// SDXL models load single-file (`--model`, encoders baked in), take a
    /// standalone fp16-fix VAE, and do NOT understand FLUX.2 Kontext
    /// reference conditioning (`-r`) — `build_sd_args` branches on this.
    pub fn is_sdxl(self) -> bool {
        matches!(self, Self::Animagine40 | Self::RealVisXl50)
    }

    /// sd-cli `--sampling-method` per checkpoint. Animagine's card says
    /// Euler Ancestral; RealVis' says DPM++ SDE (Karras); everything else
    /// keeps the turbo-default euler the pool always used.
    pub fn sampler(self) -> &'static str {
        match self {
            Self::Animagine40 => "euler_a",
            Self::RealVisXl50 => "dpm++2m_sde",
            Self::Klein4B | Self::ZImageTurbo | Self::KleinBase4B => "euler",
        }
    }

    /// sd-cli `--scheduler` override; `None` = the binary's default
    /// (discrete), which is right for everything except RealVis' Karras.
    pub fn scheduler(self) -> Option<&'static str> {
        match self {
            Self::RealVisXl50 => Some("karras"),
            Self::Klein4B | Self::ZImageTurbo | Self::KleinBase4B | Self::Animagine40 => None,
        }
    }

    /// Render resolution (W, H). SDXL is 1024-base-trained — 512×768
    /// produces duplicated anatomy, so the SDXL models use the standard
    /// 832×1216 portrait from Animagine's own card. The FLUX/Z-Image trio
    /// keeps the pool's original 512×768.
    pub fn dimensions(self) -> (u32, u32) {
        if self.is_sdxl() {
            (832, 1216)
        } else {
            (512, 768)
        }
    }

    /// sd-cli `-n` negative prompt. Only meaningful at real CFG (a cfg-1.0
    /// distilled model ignores it): Animagine ships its model card's
    /// recommended negative verbatim; RealVis' card gives none (revisit at
    /// the measurement session).
    pub fn negative_prompt(self) -> Option<&'static str> {
        match self {
            Self::Animagine40 => Some(
                "lowres, bad anatomy, bad hands, text, error, missing finger, extra digits, \
                 fewer digits, cropped, worst quality, low quality, low score, bad score, \
                 average score, signature, watermark, username, blurry",
            ),
            Self::Klein4B | Self::ZImageTurbo | Self::KleinBase4B | Self::RealVisXl50 => None,
        }
    }

    /// REMAINING download size (GB, rounded up) — files already on disk
    /// (e.g. the encoder another model fetched) don't count. This is what
    /// the Settings button quotes, so it must be the incremental cost.
    /// Always reads freshly from `weights_dir()` — no caching — so a
    /// `image_local_model_delete` call that frees a shared file is
    /// reflected on the very next read (pinned by
    /// `missing_download_gb_reflects_delete_of_a_shared_file`).
    pub fn missing_download_gb(self) -> u64 {
        let Ok(dir) = weights_dir() else {
            let mib: u64 = self.manifest().iter().map(|f| f.size_mib).sum();
            return mib.div_ceil(1024);
        };
        missing_download_mib_in(&dir, self.manifest()).div_ceil(1024)
    }
}

/// Dir-parameterised core of `missing_download_gb`, split out so it
/// host-tests against a tempdir — the real `weights_dir()` is a live
/// app-data path that, on a dev machine, already holds real downloaded
/// weights, which would make test expectations depend on whatever
/// happens to be on disk right now.
fn missing_download_mib_in(dir: &Path, manifest: &[ManifestFile]) -> u64 {
    manifest
        .iter()
        .filter(|f| !file_plausible(dir, f))
        .map(|f| f.size_mib)
        .sum()
}

/// Model-side prompt conditioning applied AFTER `local::build_prompt`'s
/// style wrapping. Animagine (danbooru-trained) wants its quality tags as
/// a suffix on EVERY prompt — including `Raw` (hard rule 98 is about scene
/// wording; these are model conditioning, not style). The bare `safe`
/// rating tag is added only for non-`Raw` styles: gallery free-text is
/// un-gated user input, while a `Raw` prompt was composed by the pipeline
/// with the suggestive gate already applied — forcing `safe` there would
/// override an allowed scene.
pub fn finalize_prompt(
    model: LocalModel,
    style: crate::image::ImageStyle,
    prompt: String,
) -> String {
    if !matches!(model, LocalModel::Animagine40) {
        return prompt;
    }
    let mut p = format!("{prompt}, masterpiece, high score, great score, absurdres");
    if !matches!(style, crate::image::ImageStyle::Raw) {
        p.push_str(", safe");
    }
    p
}

/// Pure request → `sd-cli` argument mapping. `model_dir` holds the manifest
/// files; `out` is the target PNG; `init_img` (optional) is an img2img
/// reference (the portrait, for selfie identity). Kept free of I/O so it
/// unit-tests on any host. Turbo params (`--cfg-scale 1.0
/// --sampling-method euler`) are the schnell defaults for the FLUX
/// family; SDXL models override sampler/scheduler/negative prompt via
/// `model.sampler()`/`scheduler()`/`negative_prompt()`.
/// 512×768 portrait is the FLUX-family value — SDXL renders 832×1216 (the
/// card-recommended portrait resolution) via `model.dimensions()`.
/// `--strength 0.6` follows the win fork's real-hardware-tuned values (0.6
/// re-verified on macOS 2026-07-16: it holds the portrait's identity
/// without copying its pose). `steps` is user-chosen (Settings → Image
/// Models) and sanitised here, so a bad stored value can never reach the
/// CLI.
// One flat arg list on purpose: this mirrors the sd-cli command line
// 1:1, and a struct wrapper would put a layer between the code and
// the thing it has to match exactly.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn build_sd_args(
    model: LocalModel,
    model_dir: &Path,
    prompt: &str,
    out: &Path,
    init_img: Option<&Path>,
    kontext_refs: &[std::path::PathBuf],
    steps: u32,
    extra_negative: Option<&str>,
) -> Vec<String> {
    build_sd_args_with_seed(
        model,
        model_dir,
        prompt,
        out,
        init_img,
        kontext_refs,
        steps,
        extra_negative,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_sd_args_with_seed(
    model: LocalModel,
    model_dir: &Path,
    prompt: &str,
    out: &Path,
    init_img: Option<&Path>,
    kontext_refs: &[std::path::PathBuf],
    steps: u32,
    extra_negative: Option<&str>,
    seed: Option<i64>,
) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    for f in model.manifest() {
        a.push(format!("--{}", f.role));
        a.push(model_dir.join(f.filename).to_string_lossy().into_owned());
    }
    a.push("-p".into());
    a.push(prompt.to_string());
    // CFG + steps are per-model: distilled checkpoints are trained for
    // guidance 1.0 and a handful of steps; the base models need real CFG
    // and a full schedule. A steps choice stored while another model was
    // active sanitises to THIS model's default rather than leaking.
    a.push("--cfg-scale".into());
    a.push(model.cfg_scale().to_string());
    a.push("--sampling-method".into());
    a.push(model.sampler().into());
    if let Some(sched) = model.scheduler() {
        a.push("--scheduler".into());
        a.push(sched.into());
    }
    // Negative prompt = the checkpoint's own pinned terms plus whatever
    // this REQUEST needs suppressed (`ImageRequest::extra_negative` —
    // e.g. the person/hands concepts a [[PHOTO]] must not contain).
    // Checkpoints with no pinned negative still get the request's, or
    // the caller's suppression would silently do nothing on them.
    let negative: Option<String> = match (model.negative_prompt(), extra_negative) {
        (Some(pinned), Some(extra)) if !extra.trim().is_empty() => {
            Some(format!("{pinned}, {}", extra.trim()))
        }
        (Some(pinned), _) => Some(pinned.to_string()),
        (None, Some(extra)) if !extra.trim().is_empty() => Some(extra.trim().to_string()),
        (None, _) => None,
    };
    if let Some(neg) = negative {
        a.push("-n".into());
        a.push(neg);
    }
    a.push("--steps".into());
    a.push(model.sanitize_steps(steps).to_string());
    // sd-cli defaults to a fixed seed (42) when none is given, so the
    // identical prompt would render the identical image every time.
    // -1 asks it to pick a fresh random seed per render.
    a.push("--seed".into());
    a.push(seed.unwrap_or(-1).to_string());
    let (w, h) = model.dimensions();
    a.push("--width".into());
    a.push(w.to_string());
    a.push("--height".into());
    a.push(h.to_string());
    a.push("--output".into());
    a.push(out.to_string_lossy().into_owned());
    // From sd.cpp's own docs/flux2.md invocation for this checkpoint, and
    // used in the 2026-07-16 smoke test that measured 29.7 s: flash
    // attention for the diffusion pass, and offloading idle weights to
    // system RAM so a render doesn't have to hold the whole model in the
    // Metal working set alongside a resident chat model.
    a.push("--diffusion-fa".into());
    a.push("--offload-to-cpu".into());
    if let Some(p) = init_img {
        a.push("--init-img".into());
        a.push(p.to_string_lossy().into_owned());
        a.push("--strength".into());
        a.push("0.6".into());
    }
    // Kontext reference conditioning (FLUX.2): each `-r` image steers the
    // composition without being denoised into it — this is how [[LIFE]]
    // carries her identity into a NEW pose and [[US]] composes two people.
    // SDXL has no Kontext — on those models the refs are skipped and
    // LIFE/US identity rides the appearance line alone (disclosed in the
    // Settings card).
    if !model.is_sdxl() {
        for r in kontext_refs {
            a.push("-r".into());
            a.push(r.to_string_lossy().into_owned());
        }
    }
    a
}

/// The sd.cpp CLI binary name (as built by `cmake --build`, upstream
/// master). Windows carries the `.exe` extension; every other platform
/// does not. Gated rather than hardcoded because BOTH names are used as
/// literal filenames on disk — a missing `.exe` on Windows means
/// `sd_cli_path()` silently finds nothing and local image generation
/// reports "not bundled in this build" on a machine that has it.
#[cfg(windows)]
const SD_EXE: &str = "sd-cli.exe";
#[cfg(not(windows))]
const SD_EXE: &str = "sd-cli";

/// Resolve the sd-cli binary. Absolute paths only, never PATH (hard
/// rule 93): the sandboxed .app inherits a minimal PATH and must not
/// exec anything outside its own bundle anyway.
///
/// Order, most-preferred first:
///   0. (WINDOWS ONLY) the fetched, fully-provisioned GPU build in
///      app-data. Windows ships separate per-backend sd.cpp builds and
///      the accelerated ones are too large to bundle, so the GPU build
///      is downloaded on demand — see `sidecars::gpu_runtime`. This
///      returns `Some` ONLY when the `.provisioned` sentinel confirms a
///      complete runtime for this release and this GPU vendor; a partial
///      or stale one falls through to the bundled CPU build below rather
///      than shadowing it with a binary that would spawn and fail.
///   1. next to the running binary (`Contents/MacOS/sd-cli` on macOS —
///      staged + signed by scripts/package.sh)
///   2. (WINDOWS ONLY) the bundled CPU build in the `image-runtime`
///      resource dir, which on Windows sits beside the exe. Unlike the
///      macOS layout, sd.cpp's Windows build is a binary PLUS backend
///      DLLs that must stay together, so it is bundled as a DIRECTORY
///      resource rather than staged flat next to the exe.
///   3. the dev tree (`src-tauri/binaries/sd-cli`, and on Windows also
///      `src-tauri/image-runtime/`, which is where
///      `scripts/fetch-binaries-win.ps1` stages it, for `tauri dev`)
///
/// SIGNATURE IS LOAD-BEARING: every caller (`generate`, the image
/// commands) treats this as an infallible-or-`None` lookup with no
/// AppHandle and no async. The Windows branch is deliberately pure I/O
/// for the same reason — it must never download, spawn, or block here.
pub fn sd_cli_path() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        if let Some(gpu_path) = crate::sidecars::gpu_runtime::current_sd_cli_path() {
            return Some(gpu_path);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(SD_EXE);
            if p.is_file() {
                return Some(p);
            }
            #[cfg(windows)]
            {
                // Tauri's `resources` bundle is normally extracted under a
                // `resources/` directory next to the executable. Keep the
                // direct sibling fallback for dev layouts and older installers.
                for bundled in [
                    dir.join("resources").join("image-runtime").join(SD_EXE),
                    dir.join("image-runtime").join(SD_EXE),
                ] {
                    if bundled.is_file() {
                        return Some(bundled);
                    }
                }
            }
        }
    }
    let dev = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(SD_EXE);
    if dev.is_file() {
        return Some(dev);
    }
    #[cfg(windows)]
    {
        let dev_win = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("image-runtime")
            .join(SD_EXE);
        if dev_win.is_file() {
            return Some(dev_win);
        }
    }
    None
}

/// App-data dir holding the downloaded manifest weight files. Inside the
/// sandbox this lands in the container, exactly like the Ollama models.
pub fn weights_dir() -> AppResult<std::path::PathBuf> {
    Ok(crate::storage::app_data_dir()?
        .join("image-models")
        .join("weights"))
}

/// Which of `target`'s manifest filenames are safe to delete right now,
/// given whether each OTHER model is currently fully downloaded
/// (`other_ready(m)` should answer that the same way `weights_ready_for`
/// does — never called with `target` itself).
///
/// A shared filename is KEPT only when some OTHER ready model still
/// needs it — e.g. deleting Z-Image-Turbo while klein-4B is ready must
/// keep `Qwen3-4B-Q4_K_M.gguf`, or klein's next render fails. If no
/// OTHER model is ready (whether or not it lists the file at all), the
/// file is deletable. This deliberately includes the "leftover" case: a
/// shared file that happens to exist on disk but whose only other owner
/// isn't fully downloaded gets deleted too — nothing currently depends
/// on it, and re-fetching it later (if the user eventually downloads
/// that other model) is cheap next to permanently stranding disk space.
/// Pure and I/O-free so it host-tests without touching a real weights
/// dir; the caller supplies "is model X ready" from `weights_ready_for`.
pub fn plan_weight_deletion(
    target: LocalModel,
    other_ready: impl Fn(LocalModel) -> bool,
) -> Vec<&'static str> {
    target
        .manifest()
        .iter()
        .filter(|f| {
            !LocalModel::CHOICES.iter().any(|&m| {
                m != target
                    && other_ready(m)
                    && m.manifest().iter().any(|of| of.filename == f.filename)
            })
        })
        .map(|f| f.filename)
        .collect()
}

/// True when ANY of `target`'s manifest files currently has an
/// in-progress `.part` download — `part_exists(filename)` should answer
/// "does `<filename>.part` exist in the weights dir right now" (real
/// I/O lives in the caller). Because `.part` files are named after the
/// shared FILENAME rather than any one model, this is automatically
/// shared-download-aware: if another model's in-flight download is
/// currently writing a file `target` also lists, this reports true and
/// the delete must be blocked (deleting mid-write risks a corrupt read
/// on either model's side, and racing `download_model`'s rename could
/// clobber a partial write).
pub fn download_in_progress_for(target: LocalModel, part_exists: impl Fn(&str) -> bool) -> bool {
    target.manifest().iter().any(|f| part_exists(f.filename))
}

/// Pure decision: should deleting `target`'s weights be blocked right
/// now? Unlike `commands::models::model_delete_block_reason` (Ollama),
/// being the currently SELECTED image model is deliberately NOT a block
/// reason here — re-selecting and re-downloading is a normal, supported
/// flow, and `generate()` already errors cleanly via `weights_ready_for`
/// when a file is missing rather than silently degrading. The two real
/// unsafe windows are: a render in flight (sd-cli may have any of these
/// files open/mmap'd right now) and an in-progress `.part` write for one
/// of the same files (this model's own download, or another model's
/// download of a file they share).
pub fn weight_delete_block_reason(
    render_in_progress: bool,
    download_in_progress: bool,
) -> Option<&'static str> {
    if render_in_progress {
        return Some(
            "a local render is in progress — wait for it to finish before deleting weights",
        );
    }
    if download_in_progress {
        return Some("a download is in progress for one of these files — wait for it to finish or cancel it first");
    }
    None
}

/// A file counts as present only when it exists AND is at least half its
/// manifest size — an HF-CDN error page saved as the file (observed live
/// 2026-07-16, a 936-byte HTML 504) must read as missing, not done.
fn file_plausible(dir: &Path, f: &ManifestFile) -> bool {
    let p = dir.join(f.filename);
    let min_bytes = (f.size_mib.saturating_mul(1024 * 1024)) / 2;
    p.metadata()
        .map(|m| m.len() >= min_bytes.max(1))
        .unwrap_or(false)
        && !dir.join(format!("{}.part", f.filename)).exists()
}

fn verification_cache_path(dir: &Path, f: &ManifestFile) -> std::path::PathBuf {
    dir.join(format!("{}.sha256", f.filename))
}

fn verify_file_hash(dir: &Path, f: &ManifestFile) -> bool {
    if f.sha256.is_empty() {
        return false;
    }
    let path = dir.join(f.filename);
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let cache = verification_cache_path(dir, f);
    if let Ok(value) = std::fs::read_to_string(&cache) {
        let mut fields = value.split_whitespace();
        if fields.next() == Some(f.sha256)
            && fields.next().and_then(|value| value.parse::<u64>().ok()) == Some(metadata.len())
            && fields.next().and_then(|value| value.parse::<u64>().ok()) == Some(modified)
        {
            return true;
        }
    }
    let Ok(mut file) = std::fs::File::open(&path) else {
        return false;
    };
    use sha2::Digest;
    use std::io::Read;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let Ok(read) = file.read(&mut buffer) else {
            return false;
        };
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != f.sha256 {
        return false;
    }
    let _ = std::fs::write(
        cache,
        format!("{} {} {}\n", f.sha256, metadata.len(), modified),
    );
    true
}

/// True when every manifest file is present and non-partial.
pub fn weights_ready_for(model: LocalModel) -> bool {
    let Ok(dir) = weights_dir() else {
        return false;
    };
    model
        .manifest()
        .iter()
        .all(|f| file_plausible(&dir, f) && verify_file_hash(&dir, f))
}

/// Serializes local renders — one FLUX run peaks at multi-GB memory, and
/// desktop selfies render in SPAWNED tasks, so a selfie plus a Test
/// Generation click could otherwise overlap. Requests queue here; the
/// second render starts when the first finishes.
static LOCAL_RENDER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static IMAGE_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Serializes weight downloads across models because manifests share encoder
/// and VAE files. Concurrent `.part` writes are not safe.
static LOCAL_DOWNLOAD_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static DOWNLOAD_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Try to claim the render slot WITHOUT waiting — used by weight
/// deletion so it can't silently race a render that has these files
/// open (unlike a real render, a delete should fail loudly instead of
/// queuing behind one, since the user is expecting an instant file
/// operation). `None` means a render is currently in flight; `Some`
/// hands back the guard, which the caller must hold for the whole
/// delete so a NEW render can't start mid-delete either.
pub fn try_render_slot() -> Option<tokio::sync::MutexGuard<'static, ()>> {
    LOCAL_RENDER_LOCK.try_lock().ok()
}

pub fn try_download_slot() -> Option<tokio::sync::MutexGuard<'static, ()>> {
    LOCAL_DOWNLOAD_LOCK.try_lock().ok()
}

pub fn cancel_generation() {
    IMAGE_CANCEL_REQUESTED.store(true, Ordering::SeqCst);
}

fn take_cancel_request(request: &AtomicBool) -> bool {
    request.swap(false, Ordering::SeqCst)
}

pub fn cancel_download() {
    DOWNLOAD_CANCEL_REQUESTED.store(true, Ordering::SeqCst);
}

/// Render an image locally via sd.cpp.
pub async fn generate(req: crate::image::ImageRequest) -> AppResult<crate::image::ImageResult> {
    use base64::Engine;

    let _render_slot = LOCAL_RENDER_LOCK.lock().await;
    // A request made while another render holds the slot must survive the
    // wait. Consume it only after acquiring the slot, before doing any work.
    if take_cancel_request(&IMAGE_CANCEL_REQUESTED) {
        return Err(AppError::Other("sd-cli render cancelled".into()));
    }

    let cli = sd_cli_path().ok_or_else(|| {
        AppError::NotImplemented(
            "local image engine (stable-diffusion.cpp) is not bundled in this build",
        )
    })?;
    // Snapshot the GPU runtime's provisioning generation at the same
    // instant `cli` was resolved — not later, at the point of a failure.
    // If a boot/retry provisioning pass commits a NEW runtime while THIS
    // render is still in flight against the OLD one, a failure below must
    // invalidate the runtime that actually failed, not whatever happens to
    // be current when the error is handled. See
    // `gpu_runtime::invalidate_current_runtime`'s doc comment.
    #[cfg(windows)]
    let gpu_generation_at_spawn = crate::sidecars::gpu_runtime::current_generation();
    // Unrecognised ids (hand-edited app_meta, an id a future build
    // retired) fall back to the default rather than failing the render.
    let model = req
        .local_model
        .as_deref()
        .and_then(LocalModel::from_id)
        .unwrap_or_default();
    if !weights_ready_for(model) {
        return Err(AppError::Config(
            "local image model not downloaded — open Settings → Models → Image Models → Download"
                .into(),
        ));
    }
    let mdir = weights_dir()?;
    let temp_dir = crate::storage::renderer_temp_dir()?;
    let out = temp_dir.join(format!("fpv_img_{}.png", uuid::Uuid::new_v4().simple()));

    // Optional img2img reference for identity-guided generation.
    let init_tmp = match req.reference_image_b64.as_deref() {
        Some(b64) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| AppError::Other(format!("reference image decode: {e}")))?;
            let p = temp_dir.join(format!("fpv_ref_{}.png", uuid::Uuid::new_v4().simple()));
            tokio::fs::write(&p, &bytes)
                .await
                .map_err(|e| AppError::Other(format!("reference image write: {e}")))?;
            Some(p)
        }
        None => None,
    };

    // Kontext refs land as temp files, same lifecycle as the init image.
    let mut ref_tmps: Vec<std::path::PathBuf> = Vec::new();
    for b64 in &req.kontext_refs_b64 {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| AppError::Other(format!("kontext ref decode: {e}")))?;
        let p = temp_dir.join(format!("fpv_ref_{}.png", uuid::Uuid::new_v4().simple()));
        tokio::fs::write(&p, &bytes)
            .await
            .map_err(|e| AppError::Other(format!("kontext ref write: {e}")))?;
        ref_tmps.push(p);
    }
    if model.is_sdxl() && !ref_tmps.is_empty() {
        tracing::info!(
            model = model.id(),
            n = ref_tmps.len(),
            "SDXL model: dropping kontext ref(s), identity rides the appearance line only"
        );
    }

    let prompt = finalize_prompt(model, req.style, crate::image::local::build_prompt(&req));
    let steps = model.sanitize_steps(req.local_steps.unwrap_or(model.default_steps()));
    let seed = req
        .seed
        .unwrap_or_else(|| rand::random::<u64>() as i64 & i64::MAX);
    let args = build_sd_args_with_seed(
        model,
        &mdir,
        &prompt,
        &out,
        init_tmp.as_deref(),
        &ref_tmps,
        steps,
        req.extra_negative.as_deref(),
        Some(seed),
    );

    // The request may arrive while weights, references, or prompt arguments
    // are being prepared. Check again immediately before process startup;
    // requests arriving after this point are handled by the select below.
    if take_cancel_request(&IMAGE_CANCEL_REQUESTED) {
        cleanup_temp_files(init_tmp, ref_tmps).await;
        return Err(AppError::Other("sd-cli render cancelled".into()));
    }

    // Reset first so a new render's chip starts at 0, then stream the
    // sampling bar (sd-cli prints it to stdout; logs go to stderr — pump
    // both, the parser ignores non-bar lines) into the progress channel.
    render_progress_tx().send_replace(0);
    let mut child = tokio::process::Command::new(&cli)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            // A spawn failure is never a normal render outcome — the process
            // did not start at all. On Windows that is exactly how a broken
            // fetched GPU runtime presents: CreateProcess resolves the exe's
            // import table first, so a missing/incompatible backend DLL fails
            // here (ERROR_MOD_NOT_FOUND) rather than at exit. Withdraw the
            // runtime's "provisioned" claim so the NEXT render falls back to
            // the bundled CPU build instead of failing forever. Only for the
            // fetched runtime; the bundled build has no fallback to give.
            #[cfg(windows)]
            {
                if crate::sidecars::gpu_runtime::is_gpu_runtime_path(&cli) {
                    // Return value intentionally unused: `invalidate_current_runtime`
                    // already logs (warn on success, error on a real failure to
                    // remove the sentinel) — nothing more to surface here, and
                    // this spawn error is already being returned to the caller.
                    // The generation snapshot ensures this only downgrades the
                    // exact runtime `cli` pointed at, not whatever a concurrent
                    // provisioning pass has since committed.
                    crate::sidecars::gpu_runtime::invalidate_current_runtime(
                        gpu_generation_at_spawn,
                    );
                }
            }
            AppError::Other(format!("sd-cli spawn failed: {e}"))
        })?;
    let pump_out = child.stdout.take().map(|p| tokio::spawn(pump_progress(p)));
    let pump_err = child.stderr.take().map(|p| tokio::spawn(pump_progress(p)));
    // A hung sd-cli holds LOCAL_RENDER_LOCK forever otherwise — every
    // later render queues behind it and weight-deletion is blocked until
    // the app restarts. SDXL renders measured ~212 s (hard rule 105/106
    // history), so 15 min is generous while still failing eventually.
    // Both branches below must still clean up the decoded portrait/
    // kontext temp files — the old bare `?` skipped that on a wait error.
    const RENDER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);
    let status = tokio::select! {
        result = child.wait() => match result {
            Ok(status) => status,
            Err(error) => {
                let _ = tokio::fs::remove_file(&out).await;
                for pump in [pump_out, pump_err].into_iter().flatten() { pump.abort(); }
                cleanup_temp_files(init_tmp, ref_tmps).await;
                return Err(AppError::Other(format!("sd-cli wait failed: {error}")));
            }
        },
        _ = tokio::time::sleep(RENDER_TIMEOUT) => {
            let _ = child.kill().await;
            let _ = tokio::fs::remove_file(&out).await;
            for pump in [pump_out, pump_err].into_iter().flatten() { pump.abort(); }
            cleanup_temp_files(init_tmp, ref_tmps).await;
            return Err(AppError::Other("sd-cli render timed out".into()));
        }
        _ = async {
            while !IMAGE_CANCEL_REQUESTED.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        } => {
            let _ = child.kill().await;
            let _ = tokio::fs::remove_file(&out).await;
            for pump in [pump_out, pump_err].into_iter().flatten() { pump.abort(); }
            cleanup_temp_files(init_tmp, ref_tmps).await;
            take_cancel_request(&IMAGE_CANCEL_REQUESTED);
            return Err(AppError::Other("sd-cli render cancelled".into()));
        }
    };
    for pump in [pump_out, pump_err].into_iter().flatten() {
        let _ = pump.await;
    }
    render_progress_tx().send_replace(100);
    cleanup_temp_files(init_tmp, ref_tmps).await;
    if !status.success() {
        // A normal render failure (bad params, OOM, a malformed model, a
        // cancelled-but-not-caught run, ...) exits non-zero with an
        // application-chosen code and must NOT trigger a runtime downgrade.
        // But on Windows, a missing/incompatible backend DLL does not always
        // fail at spawn(): CreateProcess can succeed and the process then
        // dies while the loader resolves imports, exiting with one of a
        // small set of NTSTATUS values reinterpreted as a process exit code.
        // These are loader-generated, not something sd-cli itself ever
        // returns, so matching them here cannot false-positive on a genuine
        // render failure.
        #[cfg(windows)]
        {
            const STATUS_DLL_NOT_FOUND: i32 = 0xC0000135u32 as i32;
            const STATUS_DLL_INIT_FAILED: i32 = 0xC0000142u32 as i32;
            const STATUS_ENTRYPOINT_NOT_FOUND: i32 = 0xC0000139u32 as i32;
            if matches!(
                status.code(),
                Some(STATUS_DLL_NOT_FOUND)
                    | Some(STATUS_DLL_INIT_FAILED)
                    | Some(STATUS_ENTRYPOINT_NOT_FOUND)
            ) && crate::sidecars::gpu_runtime::is_gpu_runtime_path(&cli)
            {
                // Same reasoning as the spawn() error path above: withdraw
                // the runtime's "provisioned" claim so the NEXT render falls
                // back to the bundled CPU build instead of failing forever.
                // Return value intentionally unused — the function already
                // logs internally, and this render is already erroring out.
                // Same generation snapshot as the spawn() path, for the same
                // reason.
                crate::sidecars::gpu_runtime::invalidate_current_runtime(
                    gpu_generation_at_spawn,
                );
            }
        }
        let _ = tokio::fs::remove_file(&out).await;
        return Err(AppError::Other(format!(
            "sd-cli exited with status {status}"
        )));
    }

    let bytes = match tokio::fs::read(&out).await {
        Ok(b) => b,
        Err(e) => {
            let _ = tokio::fs::remove_file(&out).await;
            return Err(AppError::Other(format!("sd-cli output read: {e}")));
        }
    };
    let _ = tokio::fs::remove_file(&out).await;
    validate_png(&bytes)?;
    let image_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    use sha2::Digest;
    let mut prompt_hasher = sha2::Sha256::new();
    prompt_hasher.update(prompt.as_bytes());
    Ok(crate::image::ImageResult {
        image_b64,
        provider: "local".into(),
        model: model.id().into(),
        seed,
        steps,
        cfg_scale: model.cfg_scale(),
        width: model.dimensions().0,
        height: model.dimensions().1,
        sampler: model.sampler().into(),
        scheduler: model.scheduler().map(str::to_string),
        prompt_hash: format!("{:x}", prompt_hasher.finalize()),
    })
}

/// Aggregate percent (0..=100) of `done`/`total` MiB, clamped.
fn pct(done: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    ((done.min(total) * 100) / total) as u8
}

/// One-time download of the given `model`'s manifest into `weights_dir()`,
/// reporting aggregate progress through `on_progress(percent, message)`
/// (the caller — `image::local::prewarm` — forwards to the
/// `image:prewarm` Tauri event). Each file lands atomically (`.part` →
/// rename), is skipped if already present, and resumes an interrupted
/// `.part` at BYTE granularity via an HTTP `Range` request. `HF_TOKEN`
/// (env) is sent as a bearer when set; the default manifest needs none.
pub async fn download_model<F>(model: LocalModel, mut on_progress: F) -> AppResult<()>
where
    F: FnMut(u8, &str) + Send,
{
    let _download_slot = LOCAL_DOWNLOAD_LOCK.lock().await;
    DOWNLOAD_CANCEL_REQUESTED.store(false, Ordering::SeqCst);
    let manifest = model.manifest();
    use futures_util::StreamExt;
    use std::io::Write;

    let dir = weights_dir()?;
    std::fs::create_dir_all(&dir)?;
    let hf_token = std::env::var("HF_TOKEN").ok().filter(|s| !s.is_empty());
    let total_mib: u64 = manifest.iter().map(|f| f.size_mib).sum();

    // Free-disk guard (fail fast instead of filling the disk mid-download).
    // Only counts files not already present; +5 GiB margin. Best-effort —
    // if free space can't be read we proceed rather than block (fail-open).
    let remaining_mib: u64 = manifest
        .iter()
        .filter(|f| !verify_file_hash(&dir, f))
        .map(|f| f.size_mib)
        .sum();
    let need_gib = remaining_mib / 1024 + 5;
    if let Some(free_gib) = crate::image::local::free_disk_gb_at(&dir) {
        if free_gib < need_gib {
            return Err(AppError::Config(format!(
                "not enough free disk for the local image model: need ~{need_gib} GB, {free_gib} GB free"
            )));
        }
    }

    // A single total request timeout would kill a legitimately-slow
    // multi-GB download (hard rule 19). Use an idle read timeout instead.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(120))
        .build()?;

    let mut done_mib: u64 = 0;
    for f in manifest {
        let final_path = dir.join(f.filename);
        // A pre-existing file that is implausibly small is junk from a
        // failed earlier attempt (e.g. an HF-CDN error page saved as the
        // file) — redownload it rather than trusting presence alone.
        if final_path.exists() {
            if verify_file_hash(&dir, f) {
                done_mib += f.size_mib;
                continue;
            }
            let _ = std::fs::remove_file(&final_path);
            let _ = std::fs::remove_file(verification_cache_path(&dir, f));
        }
        let part = dir.join(format!("{}.part", f.filename));
        let mut resume_from = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);

        let mut req = client.get(f.url);
        if let Some(tok) = hf_token.as_deref() {
            req = req.header("Authorization", format!("Bearer {tok}"));
        }
        if resume_from > 0 {
            req = req.header("Range", format!("bytes={resume_from}-"));
        }
        let resp = req.send().await?;
        let status = resp.status();

        // 416 = our `.part` is already ≥ the full size (stale/corrupt):
        // wipe it so the next attempt retries this file cleanly.
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            let _ = std::fs::remove_file(&part);
        }
        if !status.is_success() {
            let hint = if matches!(status.as_u16(), 401 | 403) {
                " — this file needs a Hugging Face token; set the HF_TOKEN env var"
            } else {
                ""
            };
            on_progress(
                pct(done_mib, total_mib),
                &format!("Download failed for {}{}", f.filename, hint),
            );
            return Err(AppError::Other(format!(
                "image model download {} for {}{}",
                status, f.filename, hint
            )));
        }

        // Only genuinely resuming when the server honoured the Range (206).
        // A plain 200 means it re-sent the whole file → restart the `.part`.
        let resuming = resume_from > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
        if !resuming {
            resume_from = 0;
        }
        let file_total = resp.content_length().map(|c| resume_from + c);

        let mut file = if resuming {
            std::fs::OpenOptions::new().append(true).open(&part)?
        } else {
            std::fs::File::create(&part)?
        };
        let mut got: u64 = resume_from;
        let mut stream = resp.bytes_stream();
        let msg = format!("Downloading {}", f.filename);
        // Throttle progress to CHANGES of the integer percent. The stream
        // yields ~1000 × ~15 KB chunks per second at full speed (measured
        // 2026-07-17: 17.5 MB/s bare), and the callback behind this is a
        // Tauri event emit — emitting per chunk flooded the webview and
        // throttled the download itself to ~6 KB/s (a ~1000× slowdown,
        // observed live). One callback per percent = ≤100 per download.
        let mut last_pct = u8::MAX;
        while let Some(chunk) = stream.next().await {
            if DOWNLOAD_CANCEL_REQUESTED.load(Ordering::SeqCst) {
                return Err(AppError::Other(
                    "image model download cancelled; partial file kept for resume".into(),
                ));
            }
            let chunk = chunk?;
            file.write_all(&chunk)?;
            got += chunk.len() as u64;
            let this_mib = file_total
                .map(|c| (got as f64 / c.max(1) as f64) * f.size_mib as f64)
                .unwrap_or(0.0);
            let p = pct(done_mib + this_mib as u64, total_mib);
            if p != last_pct {
                last_pct = p;
                on_progress(p, &msg);
            }
        }
        file.flush()?;
        std::fs::rename(&part, &final_path)?;
        // A 200/206 status doesn't guarantee a real file — hard rule 107
        // observed a live 2026-07-16 HF-CDN outage where a 504 landed as a
        // ~936-byte HTML page saved WITH a success status. Without this
        // check, a fresh (or just-resumed) download that hit that failure
        // mode would still report "Local image model ready" even though
        // the very next `weights_ready_for` call disagrees — reuse the
        // exact half-size test `file_plausible` already applies to
        // pre-existing files, right after the rename, so the two never
        // disagree.
        if !file_plausible(&dir, f) || !verify_file_hash(&dir, f) {
            let _ = std::fs::remove_file(&final_path);
            let _ = std::fs::remove_file(verification_cache_path(&dir, f));
            on_progress(
                pct(done_mib, total_mib),
                &format!(
                    "Download failed for {} (unexpectedly small — likely a CDN error page)",
                    f.filename
                ),
            );
            return Err(AppError::Other(format!(
                "image model download for {} produced an implausibly small file (possible CDN error page) — retry",
                f.filename
            )));
        }
        done_mib += f.size_mib;
    }

    on_progress(100, "Local image model ready");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn progress_line_parses_sdcpp_bar_frames() {
        assert_eq!(
            parse_progress_line("  |==>              | 1/4 - 2.10s/it"),
            Some(25)
        );
        assert_eq!(
            parse_progress_line("  |=========>       | 2/4 - 1.95s/it"),
            Some(50)
        );
        assert_eq!(
            parse_progress_line("  |=================| 4/4 - 2.02s/it"),
            Some(100)
        );
        assert_eq!(parse_progress_line("12/20 - 4.50it/s"), Some(60));
    }

    /// Frames captured VERBATIM from a real render on this machine
    /// (sd.cpp master a8a91b2, FLUX.1-schnell q4_0, M3 Pro, 2026-07-16) —
    /// the ones above are hand-written and predate ever running the
    /// binary. Note the trailing `ESC[K` (clear-to-EOL): sd.cpp appends it
    /// to every bar redraw, and the parser must not choke on it. It
    /// doesn't, because `12.80s/it\x1b[K` fails the u32 parse and only
    /// `4/4` matches — but that is worth pinning rather than assuming.
    #[test]
    fn progress_line_parses_real_captured_frames() {
        assert_eq!(
            parse_progress_line(
                "  |=========================>                        | 2/4 - 12.84s/it\u{1b}[K  "
            ),
            Some(50)
        );
        assert_eq!(
            parse_progress_line(
                "  |=====================================>            | 3/4 - 12.82s/it\u{1b}[K  "
            ),
            Some(75)
        );
        assert_eq!(
            parse_progress_line(
                "  |==================================================| 4/4 - 12.80s/it\u{1b}[K  "
            ),
            Some(100)
        );
    }

    #[test]
    fn progress_line_ignores_log_lines() {
        assert_eq!(
            parse_progress_line("[INFO ] stable-diffusion.cpp:1140 - loading model"),
            None
        );
        assert_eq!(parse_progress_line("loading tensors from model.gguf"), None);
        assert_eq!(parse_progress_line("saved at 2026/07/15"), None);
        assert_eq!(parse_progress_line("| 30/4 - it/s"), None);
        assert_eq!(parse_progress_line(""), None);
    }

    #[test]
    fn png_validation_rejects_non_png_and_invalid_dimensions() {
        assert!(validate_png(b"not an image").is_err());
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(&1u32.to_be_bytes());
        png.extend([8, 6, 0, 0, 0]);
        png.extend([0, 0, 0, 0]);
        assert!(validate_png(&png).is_err());
    }

    #[test]
    fn png_validation_accepts_bounded_ihdr() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&1024u32.to_be_bytes());
        png.extend_from_slice(&768u32.to_be_bytes());
        png.extend([8, 6, 0, 0, 0]);
        png.extend([0, 0, 0, 0]);
        png.extend([0, 0, 0, 0, b'I', b'E', b'N', b'D', 0, 0, 0, 0]);
        assert!(validate_png(&png).is_ok());
    }

    #[test]
    fn taking_active_render_cancellation_clears_the_next_render_state() {
        let request = AtomicBool::new(true);
        assert!(take_cancel_request(&request));
        assert!(!take_cancel_request(&request));
    }

    #[test]
    fn manifest_has_all_flux_roles_and_sizes() {
        let roles: Vec<&str> = LocalModel::default()
            .manifest()
            .iter()
            .map(|f| f.role)
            .collect();
        // FLUX.2 roles: one LLM text encoder, not CLIP-L + T5XXL.
        for r in ["diffusion-model", "vae", "llm"] {
            assert!(roles.contains(&r), "manifest missing role {r}");
        }
        assert!(LocalModel::default()
            .manifest()
            .iter()
            .all(|f| !f.filename.is_empty()));
        assert!(LocalModel::default()
            .manifest()
            .iter()
            .all(|f| f.url.starts_with("https://huggingface.co/")));
        // Sanity floor, not a spec: a manifest summing to ~nothing means someone
        // zeroed the sizes and broke the disk guard + progress weighting.
        // klein-4B totals ~4.8 GB (was 17.6 for FLUX.1-schnell).
        assert!(
            LocalModel::default()
                .manifest()
                .iter()
                .map(|f| f.size_mib)
                .sum::<u64>()
                > 4_000
        );
    }

    #[test]
    fn args_include_all_manifest_roles_and_turbo_params() {
        let dir = PathBuf::from("/m");
        let out = PathBuf::from("/tmp/o.png");
        let args = build_sd_args(
            LocalModel::default(),
            &dir,
            "a cat",
            &out,
            None,
            &[],
            4,
            None,
        );
        for role in ["diffusion-model", "vae", "llm"] {
            assert!(
                args.iter().any(|s| s == &format!("--{role}")),
                "missing --{role}"
            );
        }
        assert!(args.windows(2).any(|w| w[0] == "--steps" && w[1] == "4"));
        // f32::to_string renders 1.0 as "1" — sd-cli parses either.
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--cfg-scale" && w[1] == "1"));
        assert!(args.windows(2).any(|w| w[0] == "-p" && w[1] == "a cat"));
        assert!(args.windows(2).any(|w| w[0] == "--width" && w[1] == "512"));
        assert!(args.windows(2).any(|w| w[0] == "--height" && w[1] == "768"));
        assert!(!args.iter().any(|s| s == "--init-img"));
    }

    /// The RAM floor must reflect sd.cpp, not mflux. Measured peak for
    /// one klein-4B render: 8.05 GB — the old hardcoded `>= 14` (sized
    /// for mflux's 29-32 GB) would have told 16 GB Mac owners the
    /// feature was unavailable while it ran fine.
    #[test]
    fn ram_floor_fits_the_measured_peak() {
        let floor = crate::image::local_min_ram_gb();
        assert_eq!(floor, 12, "local backend is sd.cpp, not mflux");
        // FLUX family keeps the 12 GiB floor (klein peaked 8.05 GiB).
        assert_eq!(LocalModel::Klein4B.min_ram_gib(), 12);
        assert_eq!(LocalModel::ZImageTurbo.min_ram_gib(), 12);
        assert_eq!(LocalModel::KleinBase4B.min_ram_gib(), 12);
        // SDXL (fp16, 832×1216) peaked 11.2 GiB — above the 12 floor's
        // headroom, so it gets a 16 GiB floor of its own.
        assert_eq!(LocalModel::Animagine40.min_ram_gib(), 16);
        assert_eq!(LocalModel::RealVisXl50.min_ram_gib(), 16);
        // A 16 GB Mac still passes the SDXL floor; the 11.2 GiB peak fits.
        assert!(16 >= LocalModel::Animagine40.min_ram_gib());
    }

    /// The user's Settings choice reaches the CLI, and only a legal one
    /// does — `app_meta` is hand-editable. Per-model: a value stored for
    /// a distilled model must not leak into a base model (4 steps on a
    /// base model renders noise).
    #[test]
    fn steps_are_user_chosen_but_sanitised_per_model() {
        let dir = PathBuf::from("/m");
        let out = PathBuf::from("/o.png");
        let steps_of = |m: LocalModel, n: u32| {
            let a = build_sd_args(m, &dir, "p", &out, None, &[], n, None);
            let i = a.iter().position(|s| s == "--steps").expect("--steps");
            a[i + 1].clone()
        };
        for m in LocalModel::CHOICES {
            for n in m.step_choices() {
                assert_eq!(steps_of(m, n), n.to_string(), "{m:?} choice {n}");
            }
            assert_eq!(steps_of(m, 7), m.default_steps().to_string());
            assert_eq!(steps_of(m, 9999), m.default_steps().to_string());
        }
        // The concrete leak: 4 is legal for klein-4b, poison for base.
        assert_eq!(steps_of(LocalModel::KleinBase4B, 4), "20");
    }

    /// Per-model CFG reaches the CLI — distilled 1.0, base models real
    /// CFG (sd.cpp's own docs per checkpoint).
    #[test]
    fn cfg_scale_is_per_model() {
        let dir = PathBuf::from("/m");
        let out = PathBuf::from("/o.png");
        let cfg_of = |m: LocalModel| {
            let a = build_sd_args(m, &dir, "p", &out, None, &[], m.default_steps(), None);
            let i = a
                .iter()
                .position(|s| s == "--cfg-scale")
                .expect("--cfg-scale");
            a[i + 1].clone()
        };
        assert_eq!(cfg_of(LocalModel::Klein4B), "1");
        assert_eq!(cfg_of(LocalModel::ZImageTurbo), "1");
        assert_eq!(cfg_of(LocalModel::KleinBase4B), "4");
    }

    /// The shared weights dir depends on this invariant: the same
    /// filename in two manifests must mean the same URL and size —
    /// otherwise one model's download could overwrite another's file
    /// with different bytes.
    #[test]
    fn shared_filenames_are_consistent_across_manifests() {
        use std::collections::HashMap;
        let mut seen: HashMap<&str, (&str, u64)> = HashMap::new();
        for m in LocalModel::CHOICES {
            for f in m.manifest() {
                if let Some((url, mib)) = seen.get(f.filename) {
                    assert_eq!(*url, f.url, "filename {} has two URLs", f.filename);
                    assert_eq!(*mib, f.size_mib, "filename {} has two sizes", f.filename);
                } else {
                    seen.insert(f.filename, (f.url, f.size_mib));
                }
            }
        }
        // Every FLUX/Z-Image model shares the SAME LLM encoder file — that
        // is the "a second model costs only its diffusion file" promise for
        // that family. SDXL (Animagine/RealVis) is architecturally
        // different: single-file checkpoints with CLIP-L/G baked in, no
        // `llm` role at all (pinned by the `sdxl_manifests_are_single_
        // checkpoint_plus_vae` test) — its own encoder-sharing promise is
        // the fp16-fix VAE, covered by `sdxl_manifests_share_the_fp16_fix_vae`.
        for m in [
            LocalModel::Klein4B,
            LocalModel::ZImageTurbo,
            LocalModel::KleinBase4B,
        ] {
            assert!(m
                .manifest()
                .iter()
                .any(|f| f.filename == "Qwen3-4B-Q4_K_M.gguf"));
        }
    }

    #[test]
    fn img2img_adds_init_image_and_strength() {
        let args = build_sd_args(
            LocalModel::default(),
            &PathBuf::from("/m"),
            "p",
            &PathBuf::from("/o.png"),
            Some(&PathBuf::from("/portrait.png")),
            &[],
            4,
            None,
        );
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--init-img" && w[1] == "/portrait.png"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--strength" && w[1] == "0.6"));
    }

    #[test]
    fn five_models_with_stable_ids() {
        let ids: Vec<&str> = LocalModel::CHOICES.iter().map(|m| m.id()).collect();
        assert_eq!(
            ids,
            [
                "flux2-klein-4b",
                "z-image-turbo",
                "flux2-klein-base-4b",
                "animagine-xl-4",
                "realvis-xl-5"
            ]
        );
        for m in LocalModel::CHOICES {
            assert_eq!(LocalModel::from_id(m.id()), Some(m));
        }
    }

    #[test]
    fn all_manifest_roles_are_known_cli_flags() {
        // `build_sd_args` emits `--{role}` verbatim — every role must be a real
        // sd-cli long flag. "model" is the SDXL single-file load (-m/--model).
        const KNOWN: [&str; 5] = ["diffusion-model", "llm", "vae", "model", "clip_l"];
        for m in LocalModel::CHOICES {
            for f in m.manifest() {
                assert!(
                    KNOWN.contains(&f.role),
                    "unknown role {} on {}",
                    f.role,
                    m.id()
                );
            }
        }
    }

    #[test]
    fn sdxl_manifests_share_the_fp16_fix_vae() {
        // Same filename ⇒ same URL + size is the pool-wide invariant; here we
        // additionally pin that BOTH SDXL models actually use the shared VAE,
        // so the second SDXL download costs only its own checkpoint.
        let vae = |m: LocalModel| {
            m.manifest()
                .iter()
                .find(|f| f.role == "vae")
                .expect("sdxl needs a standalone vae (fp16 NaN fix)")
                .filename
        };
        assert_eq!(
            vae(LocalModel::Animagine40),
            "sdxl_vae_fp16_fix.safetensors"
        );
        assert_eq!(vae(LocalModel::Animagine40), vae(LocalModel::RealVisXl50));
    }

    #[test]
    fn sdxl_manifests_are_single_checkpoint_plus_vae() {
        for m in [LocalModel::Animagine40, LocalModel::RealVisXl50] {
            let roles: Vec<&str> = m.manifest().iter().map(|f| f.role).collect();
            assert_eq!(roles, ["model", "vae"], "{}", m.id());
            // Encoders are baked into the single file — no --llm, no clip files.
            let mib: u64 = m.manifest().iter().map(|f| f.size_mib).sum();
            assert!(mib > 6_000, "{} looks too small: {mib} MiB", m.id());
        }
    }

    #[test]
    fn sdxl_params_follow_the_model_cards() {
        // Animagine card: cfg 5, steps 28, Euler Ancestral, 832×1216.
        let a = LocalModel::Animagine40;
        assert_eq!(a.cfg_scale(), 5.0);
        assert_eq!(a.default_steps(), 28);
        assert_eq!(a.step_choices(), [20, 24, 28, 32]);
        assert_eq!(a.sampler(), "euler_a");
        assert_eq!(a.scheduler(), None);
        assert_eq!(a.dimensions(), (832, 1216));
        assert!(a.negative_prompt().unwrap().contains("bad anatomy"));

        // RealVis card: DPM++ SDE + Karras, 30+ steps; cfg 6.0 confirmed
        // by A/B at the 2026-07-17 measurement session.
        let r = LocalModel::RealVisXl50;
        assert_eq!(r.cfg_scale(), 6.0);
        assert_eq!(r.default_steps(), 30);
        assert_eq!(r.step_choices(), [24, 30, 40, 50]);
        assert_eq!(r.sampler(), "dpm++2m_sde");
        assert_eq!(r.scheduler(), Some("karras"));
        assert_eq!(r.dimensions(), (832, 1216));
        assert_eq!(r.negative_prompt(), None);
    }

    #[test]
    fn flux_family_params_are_unchanged() {
        for m in [
            LocalModel::Klein4B,
            LocalModel::ZImageTurbo,
            LocalModel::KleinBase4B,
        ] {
            assert_eq!(m.sampler(), "euler");
            assert_eq!(m.scheduler(), None);
            assert_eq!(m.dimensions(), (512, 768));
            assert_eq!(m.negative_prompt(), None);
            assert!(!m.is_sdxl());
        }
        // step_choices() went from an is_large() branch to a per-model
        // match in the SDXL change — pin the pre-existing menus with
        // HARD-CODED arrays (the sanitize test is self-referential and
        // would not catch a silently reshuffled menu).
        assert_eq!(LocalModel::Klein4B.step_choices(), [4, 8, 12, 16]);
        assert_eq!(LocalModel::ZImageTurbo.step_choices(), [4, 8, 12, 16]);
        assert_eq!(LocalModel::KleinBase4B.step_choices(), [12, 20, 28, 36]);
        assert!(LocalModel::Animagine40.is_sdxl());
        assert!(LocalModel::RealVisXl50.is_sdxl());
    }

    #[test]
    fn sdxl_steps_do_not_leak_across_models() {
        // A "4" stored while klein was active must sanitise to the SDXL
        // model's own default, not reach the CLI (hard rule 105).
        assert_eq!(LocalModel::Animagine40.sanitize_steps(4), 28);
        assert_eq!(LocalModel::RealVisXl50.sanitize_steps(4), 30);
    }

    #[test]
    fn sdxl_args_load_single_file_with_sampler_and_dims() {
        let dir = Path::new("/w");
        let out = Path::new("/tmp/o.png");
        let args = build_sd_args(
            LocalModel::Animagine40,
            dir,
            "1girl, cafe",
            out,
            None,
            &[],
            28,
            None,
        );
        let s = args.join(" ");
        // Paths are built with Path::join, so the expected strings must be
        // derived the same way — on Windows the separator is '\', not '/'.
        let model_path = dir.join("animagine-xl-4.0-opt.safetensors");
        let model_path = model_path.to_string_lossy();
        let vae_path = dir.join("sdxl_vae_fp16_fix.safetensors");
        let vae_path = vae_path.to_string_lossy();
        assert!(s.contains(&format!("--model {model_path}")));
        assert!(s.contains(&format!("--vae {vae_path}")));
        assert!(s.contains("--sampling-method euler_a"));
        assert!(s.contains("--width 832") && s.contains("--height 1216"));
        assert!(s.contains("--cfg-scale 5"));
        assert!(
            args.windows(2).any(|w| w[0] == "-n"),
            "Animagine must carry its negative prompt"
        );
        assert!(!s.contains("--scheduler"), "discrete default needs no flag");
        assert!(!s.contains("--llm") && !s.contains("--diffusion-model"));
    }

    #[test]
    fn a_request_negative_is_appended_to_the_checkpoint_negative() {
        let dir = Path::new("/tmp/m");
        let out = Path::new("/tmp/o.png");
        let args = build_sd_args(
            LocalModel::Animagine40,
            dir,
            "a mug",
            out,
            None,
            &[],
            28,
            Some("person, hands"),
        );
        let i = args
            .iter()
            .position(|a| a == "-n")
            .expect("negative flag missing");
        let neg = &args[i + 1];
        assert!(
            neg.contains("bad anatomy"),
            "checkpoint negative lost: {neg}"
        );
        assert!(
            neg.contains("person, hands"),
            "request negative lost: {neg}"
        );
    }

    #[test]
    fn a_request_negative_reaches_a_checkpoint_that_pins_none() {
        let dir = Path::new("/tmp/m");
        let out = Path::new("/tmp/o.png");
        let args = build_sd_args(
            LocalModel::RealVisXl50,
            dir,
            "a mug",
            out,
            None,
            &[],
            30,
            Some("person, hands"),
        );
        let i = args
            .iter()
            .position(|a| a == "-n")
            .expect("a request negative must appear even with no pinned one");
        assert_eq!(args[i + 1], "person, hands");
    }

    #[test]
    fn realvis_args_carry_karras_no_negative() {
        let args = build_sd_args(
            LocalModel::RealVisXl50,
            Path::new("/w"),
            "portrait photo",
            Path::new("/tmp/o.png"),
            None,
            &[],
            30,
            None,
        );
        let s = args.join(" ");
        assert!(s.contains("--sampling-method dpm++2m_sde"));
        assert!(s.contains("--scheduler karras"));
        assert!(
            !args.iter().any(|a| a == "-n"),
            "RealVis has no pinned negative yet"
        );
    }

    #[test]
    fn sdxl_drops_kontext_refs_keeps_init_img() {
        // -r is FLUX.2 Kontext conditioning — SDXL doesn't speak it. img2img
        // (--init-img, the SELFIE identity channel) works on SDXL and stays.
        let refs = vec![std::path::PathBuf::from("/tmp/her.png")];
        let init = Path::new("/tmp/portrait.png");
        let args = build_sd_args(
            LocalModel::Animagine40,
            Path::new("/w"),
            "selfie",
            Path::new("/tmp/o.png"),
            Some(init),
            &refs,
            28,
            None,
        );
        assert!(
            !args.iter().any(|a| a == "-r"),
            "kontext refs must be skipped on SDXL"
        );
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--init-img" && w[1] == "/tmp/portrait.png"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--strength" && w[1] == "0.6"));

        // …and klein still passes them (the existing behaviour, pinned).
        let flux = build_sd_args(
            LocalModel::Klein4B,
            Path::new("/w"),
            "selfie",
            Path::new("/tmp/o.png"),
            None,
            &refs,
            4,
            None,
        );
        assert!(flux
            .windows(2)
            .any(|w| w[0] == "-r" && w[1] == "/tmp/her.png"));
    }

    #[test]
    fn flux_args_are_byte_identical_to_before() {
        // Regression pin: the whole FLUX/Z-Image arg vector must not change
        // — asserted as the EXACT sequence, not substrings, so any silent
        // reorder/insertion on the FLUX path fails loudly. The path values
        // are derived with Path::join like the implementation does, so the
        // pin holds on every platform (Windows uses '\' separators).
        let dir = Path::new("/w");
        let out = Path::new("/tmp/o.png");
        let args = build_sd_args(
            LocalModel::Klein4B,
            dir,
            "a cat",
            out,
            None,
            &[],
            4,
            None,
        );
        let in_dir = |name: &str| dir.join(name).to_string_lossy().into_owned();
        assert_eq!(
            args,
            vec![
                "--diffusion-model".to_string(),
                in_dir("flux-2-klein-4b-Q4_0.gguf"),
                "--llm".to_string(),
                in_dir("Qwen3-4B-Q4_K_M.gguf"),
                "--vae".to_string(),
                in_dir("flux2_ae.safetensors"),
                "-p".to_string(),
                "a cat".to_string(),
                "--cfg-scale".to_string(),
                "1".to_string(),
                "--sampling-method".to_string(),
                "euler".to_string(),
                "--steps".to_string(),
                "4".to_string(),
                "--seed".to_string(),
                "-1".to_string(),
                "--width".to_string(),
                "512".to_string(),
                "--height".to_string(),
                "768".to_string(),
                "--output".to_string(),
                out.to_string_lossy().into_owned(),
                "--diffusion-fa".to_string(),
                "--offload-to-cpu".to_string(),
            ]
        );
    }

    #[test]
    fn animagine_prompts_gain_quality_tags_and_gallery_gets_safe() {
        use crate::image::ImageStyle;
        // Gallery free-text (non-Raw): quality tags + the `safe` rating tag —
        // an anime-native model drifts NSFW on neutral prompts without it.
        let gallery = finalize_prompt(
            LocalModel::Animagine40,
            ImageStyle::Anime,
            "cafe scene".into(),
        );
        assert!(gallery.ends_with("masterpiece, high score, great score, absurdres, safe"));
        // Skill path (Raw): the pipeline already ran the suggestive gate when
        // composing the prompt — quality tags yes, forced `safe` NO.
        let skill = finalize_prompt(
            LocalModel::Animagine40,
            ImageStyle::Raw,
            "her selfie".into(),
        );
        assert!(skill.ends_with("masterpiece, high score, great score, absurdres"));
        assert!(!skill.ends_with("safe"));
    }

    #[test]
    fn non_animagine_prompts_pass_through() {
        use crate::image::ImageStyle;
        for m in [
            LocalModel::Klein4B,
            LocalModel::ZImageTurbo,
            LocalModel::KleinBase4B,
            LocalModel::RealVisXl50,
        ] {
            let p = finalize_prompt(m, ImageStyle::Raw, "exact words".into());
            assert_eq!(p, "exact words", "{} must not rewrite prompts", m.id());
        }
    }

    // -- weight deletion (TDD: written before `plan_weight_deletion` /
    // `download_in_progress_for` / `weight_delete_block_reason` existed) --

    /// (a) No OTHER model is ready → nothing shares a *live* claim on any
    /// of the target's files, so every manifest file is safe to delete.
    /// This is also the "no shared files" case in practice for this pool
    /// (every model shares SOMETHING with a sibling; what matters is
    /// whether that sibling is actually on disk right now).
    #[test]
    fn plan_delete_returns_everything_when_no_other_model_is_ready() {
        let files = plan_weight_deletion(LocalModel::Klein4B, |_other| false);
        let mut got: Vec<&str> = files;
        got.sort_unstable();
        let mut want: Vec<&str> = LocalModel::Klein4B
            .manifest()
            .iter()
            .map(|f| f.filename)
            .collect();
        want.sort_unstable();
        assert_eq!(got, want);
    }

    /// (b) Deleting Z-Image-Turbo while klein-4B IS ready: both share the
    /// Qwen3-4B encoder file, so that filename must be KEPT (klein still
    /// needs it) while Z-Image's own diffusion file and its own VAE
    /// (flux1_ae.safetensors — NOT shared with klein's flux2_ae) are safe.
    #[test]
    fn plan_delete_skips_shared_file_when_other_model_is_ready() {
        let files = plan_weight_deletion(LocalModel::ZImageTurbo, |other| {
            other == LocalModel::Klein4B
        });
        assert!(
            !files.contains(&"Qwen3-4B-Q4_K_M.gguf"),
            "klein still needs the shared encoder"
        );
        assert!(files.contains(&"z_image_turbo-Q4_0.gguf"));
        assert!(files.contains(&"flux1_ae.safetensors"));
    }

    /// (c) Deleting Animagine while RealVis shares the fp16-fix VAE
    /// filename but RealVis is NOT ready (not downloaded, or only a
    /// leftover file exists) — deliberate choice, pinned here: we delete
    /// the shared file too. Rationale: nothing on disk actually depends
    /// on it right now, and re-fetching 319 MiB later if the user
    /// eventually downloads RealVis is cheap compared to silently
    /// stranding disk space forever. See `plan_weight_deletion`'s doc
    /// comment.
    #[test]
    fn plan_delete_removes_shared_file_when_other_model_is_not_ready() {
        let files = plan_weight_deletion(LocalModel::Animagine40, |_other| false);
        assert!(files.contains(&"sdxl_vae_fp16_fix.safetensors"));
        assert!(files.contains(&"animagine-xl-4.0-opt.safetensors"));
    }

    /// Deleting the currently-selected model is a normal, allowed flow
    /// (unlike Ollama's chat-default guard) — `plan_weight_deletion`
    /// doesn't even take "is this selected" as an input, which is itself
    /// the point: there is nothing to special-case. This also pins that
    /// the predicate is only ever asked about OTHER models: a predicate
    /// that panics on `target` must not panic (own-readiness is
    /// irrelevant — you can always delete weights you're actively using).
    #[test]
    fn plan_delete_never_queries_target_itself() {
        let files = plan_weight_deletion(LocalModel::Klein4B, |m| {
            assert_ne!(
                m,
                LocalModel::Klein4B,
                "must not ask about the target itself"
            );
            false
        });
        assert_eq!(files.len(), LocalModel::Klein4B.manifest().len());
    }

    #[test]
    fn download_in_progress_detects_any_manifest_file_with_a_part() {
        use std::collections::HashSet;
        let pending: HashSet<&str> = ["Qwen3-4B-Q4_K_M.gguf"].into_iter().collect();
        assert!(download_in_progress_for(LocalModel::Klein4B, |f| pending.contains(f)));
        assert!(download_in_progress_for(LocalModel::KleinBase4B, |f| {
            pending.contains(f)
        }));
        assert!(!download_in_progress_for(LocalModel::Animagine40, |f| {
            pending.contains(f)
        }));
    }

    #[test]
    fn download_in_progress_false_when_no_part_files() {
        assert!(!download_in_progress_for(LocalModel::Klein4B, |_f| false));
    }

    #[test]
    fn weight_delete_blocked_while_render_in_progress() {
        let reason = weight_delete_block_reason(true, false);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("render"));
    }

    #[test]
    fn weight_delete_blocked_while_download_in_progress() {
        let reason = weight_delete_block_reason(false, true);
        assert!(reason.is_some());
        assert!(reason.unwrap().to_lowercase().contains("download"));
    }

    #[test]
    fn weight_delete_allowed_when_idle() {
        assert_eq!(weight_delete_block_reason(false, false), None);
    }

    /// Pins `file_plausible`'s half-size threshold directly — previously
    /// untested in isolation despite backing both `weights_ready_for` and
    /// (as of this audit) the post-write check in `download_model` that
    /// catches a 200/206-status HF-CDN error page (hard rule 107).
    #[test]
    fn file_plausible_half_size_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let f = ManifestFile {
            role: "vae",
            filename: "boundary-test.bin",
            url: "https://example.invalid/boundary-test.bin",
            size_mib: 10,
            sha256: "",
        };
        let min_bytes = (f.size_mib * 1024 * 1024) / 2;

        // Below half → implausible (the CDN-error-page shape).
        let file = std::fs::File::create(dir.join(f.filename)).unwrap();
        file.set_len(min_bytes - 1).unwrap();
        assert!(!file_plausible(dir, &f));

        // Exactly half → plausible (boundary is inclusive).
        std::fs::File::create(dir.join(f.filename))
            .unwrap()
            .set_len(min_bytes)
            .unwrap();
        assert!(file_plausible(dir, &f));

        // Full size → plausible.
        std::fs::File::create(dir.join(f.filename))
            .unwrap()
            .set_len(f.size_mib * 1024 * 1024)
            .unwrap();
        assert!(file_plausible(dir, &f));

        // A lingering `.part` for the SAME filename makes even a full-size
        // final file read as not-yet-done (download_model shouldn't leave
        // one behind after a successful rename, but the guard is defensive).
        std::fs::File::create(dir.join(format!("{}.part", f.filename))).unwrap();
        assert!(!file_plausible(dir, &f));
    }

    #[test]
    fn verifies_manifest_sha256_and_invalidates_changed_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let f = ManifestFile {
            role: "vae",
            filename: "hash-test.bin",
            url: "https://example.invalid/hash-test.bin",
            size_mib: 1,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };
        std::fs::write(dir.join(f.filename), b"abc").unwrap();
        assert!(verify_file_hash(dir, &f));
        std::fs::write(dir.join(f.filename), b"changed").unwrap();
        assert!(!verify_file_hash(dir, &f));
    }

    /// Writes a sparse file of exactly `f.size_mib` (via `set_len`, no real
    /// disk write — near-instant and disk-free) so `file_plausible`'s size
    /// check passes without ever writing multi-GB of real bytes in a test.
    fn stage_full(dir: &Path, f: &ManifestFile) {
        let file = std::fs::File::create(dir.join(f.filename)).unwrap();
        file.set_len(f.size_mib * 1024 * 1024).unwrap();
    }

    /// `missing_download_gb`'s public form always reads the REAL app-data
    /// weights dir (`weights_dir()`), which on a dev machine already has
    /// real downloaded weights — unusable for a deterministic test. This
    /// pins the dir-parameterised core against a tempdir instead.
    ///
    /// Reproduces the exact interaction the 2026-07-18 storage audit asked
    /// about: `image_local_model_delete` removes a SHARED file (the Qwen3
    /// encoder) when deleting Z-Image-Turbo, because Klein-4B — which also
    /// needs that same file — isn't fully downloaded yet. After the
    /// delete, `missing_download_gb` for Klein-4B must count the shared
    /// file as missing again, not still trust its stale "present" reading.
    #[test]
    fn missing_download_gb_reflects_delete_of_a_shared_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        // Z-Image-Turbo is fully "downloaded" (all 3 manifest files staged).
        for f in LocalModel::ZImageTurbo.manifest() {
            stage_full(dir, f);
        }
        // Klein-4B is NOT downloaded — its own diffusion + VAE files are
        // absent — but it shares the Qwen3 encoder file with Z-Image-Turbo,
        // which is therefore already present on disk.
        let klein = LocalModel::Klein4B;
        assert!(
            klein
                .manifest()
                .iter()
                .any(|f| f.filename == "Qwen3-4B-Q4_K_M.gguf"),
            "test assumption: klein shares the Qwen3 encoder"
        );

        let ready = |m: LocalModel| m.manifest().iter().all(|f| file_plausible(dir, f));
        assert!(
            ready(LocalModel::ZImageTurbo),
            "z-image should read as ready"
        );
        assert!(
            !ready(klein),
            "klein should NOT read as ready (own files missing)"
        );

        let missing_before = missing_download_mib_in(dir, klein.manifest());
        let klein_own_mib: u64 = klein
            .manifest()
            .iter()
            .filter(|f| f.filename != "Qwen3-4B-Q4_K_M.gguf")
            .map(|f| f.size_mib)
            .sum();
        assert_eq!(
            missing_before, klein_own_mib,
            "before delete: only klein's own files are missing, the shared encoder is present"
        );

        // Delete Z-Image-Turbo's weights exactly as `image_local_model_delete`
        // does: plan which files are safe (shared files kept only when some
        // OTHER ready model needs them — klein isn't ready, so the shared
        // Qwen3 file is deletable too), then actually remove them.
        let safe = plan_weight_deletion(LocalModel::ZImageTurbo, ready);
        assert!(
            safe.contains(&"Qwen3-4B-Q4_K_M.gguf"),
            "shared encoder must be planned for deletion — no OTHER ready model needs it"
        );
        for filename in &safe {
            std::fs::remove_file(dir.join(filename)).unwrap();
        }

        let missing_after = missing_download_mib_in(dir, klein.manifest());
        let qwen_mib = klein
            .manifest()
            .iter()
            .find(|f| f.filename == "Qwen3-4B-Q4_K_M.gguf")
            .unwrap()
            .size_mib;
        assert_eq!(
            missing_after,
            missing_before + qwen_mib,
            "after the shared file is deleted, missing_download_gb must count it again"
        );
    }
}
