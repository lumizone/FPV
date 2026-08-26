//! Local image generation — the module's PUBLIC SURFACE, delegating to
//! `image::sdcpp` (the bundled stable-diffusion.cpp binary).
//!
//! History: until v1.5.0 this module ran **mflux** (a `uv tool install`
//! Python/MLX sidecar rendering the ~31 GB Z-Image-Turbo checkpoint at
//! 29-32 GB peak RAM). The desktop app now uses the Windows fork's sd.cpp
//! backend instead of the old mflux flow, and sd.cpp turned out better
//! on every axis (4.8 GB default download, ~30 s renders, 8 GB peak,
//! selectable checkpoints, Kontext refs for the LIFE/US skills). v1.5.1
//! retires mflux in the direct build too; both editions now render
//! through `sdcpp`. The delegation lives HERE, at the public surface, so
//! every caller (chat_pipeline::selfies_available, system_status, the
//! image commands, image::generate's dispatch) is backend-agnostic.

use serde::Serialize;

use crate::error::AppResult;

/// Returns (local_engine_installed, ram_gb). Uncached — used by the
/// Settings → Image Models check, which must reflect a fresh state
/// immediately. "Installed" = the bundled sd-cli is present, which is
/// always true in a real bundle (false means a broken/dev build).
pub fn check() -> (bool, u64) {
    (
        crate::image::sdcpp::sd_cli_path().is_some(),
        system_ram_gb(),
    )
}

pub async fn generate(req: super::ImageRequest) -> AppResult<super::ImageResult> {
    crate::image::sdcpp::generate(req).await
}

/// Weight readiness for a SPECIFIC checkpoint — weights are per-manifest,
/// so "are we ready" depends on which model is selected. The manifest
/// check is exact (named files), so this is never `None`-unknown; the
/// Option shape survives from the mflux era, whose HF-cache heuristic
/// could genuinely not know.
/// Runs off the async runtime: the underlying manifest check hashes
/// multi-GB weight files whenever the verification cache is cold.
pub async fn weights_status_of_async(model: crate::image::sdcpp::LocalModel) -> Option<bool> {
    Some(crate::image::sdcpp::weights_ready_for_async(model).await)
}

/// Coarse progress phases emitted while pre-warming. sd.cpp's pre-warm is
/// a pure download (the manifest check is exact, no validation render
/// needed), so only `Downloading` exists — mflux's `Rendering` phase went
/// with it; the frontend still accepts the string for compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrewarmPhase {
    /// Weight download in progress.
    Downloading,
}

/// Download the selected model's weights ahead of time (Settings →
/// Models → Image Models). Resumable at byte granularity; each file
/// lands atomically. Progress is forwarded to `on_progress(phase,
/// percent)` — the caller plumbs it to the `image:prewarm` Tauri event.
pub async fn prewarm<F>(model: crate::image::sdcpp::LocalModel, mut on_progress: F) -> AppResult<()>
where
    F: FnMut(PrewarmPhase, Option<u8>) + Send,
{
    crate::image::sdcpp::download_model(model, |percent, _msg| {
        on_progress(PrewarmPhase::Downloading, Some(percent))
    })
    .await
}

/// Free space (GB) on the volume holding `path` (walks up to the nearest
/// existing ancestor — the weights dir may not exist before the first
/// download). `None` when it can't be determined (fail-open; the download
/// itself will surface a disk-full error eventually).
pub(crate) fn free_disk_gb_at(path: &std::path::Path) -> Option<u64> {
    let mut probe = path;
    while !probe.exists() {
        probe = probe.parent()?;
    }
    let out = std::process::Command::new("df")
        .args(["-Pk"])
        .arg(probe)
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    // POSIX -P format: header, then one line; field 4 = available 1K blocks.
    let line = text.lines().nth(1)?;
    let avail_kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(avail_kb / (1024 * 1024))
}

fn system_ram_gb() -> u64 {
    std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|bytes| bytes / (1024 * 1024 * 1024))
        .unwrap_or(0)
}

pub(crate) fn build_prompt(req: &super::ImageRequest) -> String {
    match req.style {
        super::ImageStyle::Anime => format!("anime illustration, {}, detailed, vibrant, high quality", req.prompt),
        super::ImageStyle::Manga => format!("black and white manga panel, expressive linework, screentone, {}, high quality", req.prompt),
        super::ImageStyle::Watercolor => format!("atmospheric watercolor storybook illustration, soft pigment edges, {}, high quality", req.prompt),
        super::ImageStyle::Ink => format!("ink wash illustration, dramatic brushwork, textured paper, {}, high quality", req.prompt),
        super::ImageStyle::Cinematic => format!("cinematic film still, controlled lighting, strong composition, shallow depth of field, {}, high quality", req.prompt),
        super::ImageStyle::DarkFantasy => format!("dark fantasy concept art, moody lighting, gothic atmosphere, intricate detail, {}, high quality", req.prompt),
        super::ImageStyle::Photo | super::ImageStyle::Realistic => format!("photorealistic editorial still, natural lighting, realistic materials, {}, high quality", req.prompt),
        // Raw passes a complete prompt through verbatim.
        super::ImageStyle::Raw => req.prompt.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{ImageRequest, ImageStyle};

    #[test]
    fn ram_detection_returns_nonzero() {
        // On any Mac this should return something sane.
        let gb = system_ram_gb();
        assert!(gb > 0, "sysctl hw.memsize returned 0");
    }

    #[test]
    fn check_returns_tuple() {
        let (installed, ram) = check();
        // Just verify it doesn't panic.
        let _ = installed;
        assert!(ram > 0);
    }

    #[test]
    fn raw_style_passes_prompt_through_verbatim() {
        // Raw style must not wrap the prompt — the frontend sends a
        // complete prompt when it needs one.
        let req = ImageRequest {
            prompt: "anime male character, short black hair, rooftop at dusk".into(),
            style: ImageStyle::Raw,
            reference_image_b64: None,
            local_steps: None,
            local_model: None,
            kontext_refs_b64: Vec::new(),
            extra_negative: None,
            seed: None,
        };
        let p = build_prompt(&req);
        assert_eq!(p, req.prompt);
        assert!(
            !p.contains("anime girl"),
            "Raw must not inject a girl prefix: {p}"
        );
    }

    #[test]
    fn anime_style_still_wraps_for_gallery_prompts() {
        let req = ImageRequest {
            prompt: "silver hair, window light".into(),
            style: ImageStyle::Anime,
            reference_image_b64: None,
            local_steps: None,
            local_model: None,
            kontext_refs_b64: Vec::new(),
            extra_negative: None,
            seed: None,
        };
        let p = build_prompt(&req);
        assert!(p.contains("silver hair"));
        assert!(
            p.to_ascii_lowercase().contains("anime"),
            "gallery prompts keep the style hint"
        );
    }

    #[tokio::test]
    async fn weights_status_is_never_unknown() {
        // The manifest check is exact — callers rely on Some(_) so the
        // "unknown → fail-open" branch (an mflux relic) never fires.
        assert!(
            weights_status_of_async(crate::image::sdcpp::LocalModel::default())
                .await
                .is_some()
        );
    }
}
