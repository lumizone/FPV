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
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let mut probe = path;
        while !probe.exists() {
            probe = probe.parent()?;
        }
        let wide: Vec<u16> = probe.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free_avail: u64 = 0;
        let mut total: u64 = 0;
        let mut free_total: u64 = 0;
        // SAFETY: `wide` is a null-terminated UTF-16 buffer; the output
        // pointers point at valid u64 locals the API is required to fill.
        let ok = unsafe {
            GetDiskFreeSpaceExW(wide.as_ptr(), &mut free_avail, &mut total, &mut free_total)
        };
        if ok == 0 {
            return None;
        }
        Some(free_avail / (1024 * 1024 * 1024))
    }

    #[cfg(not(windows))]
    {
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
}

/// Total physical RAM in whole GB (rounds down). Enforces each local
/// model's minimum-RAM requirement before a render starts
/// (`commands/image.rs`) and feeds the Settings → Image Models check.
/// macOS reads `sysctl hw.memsize`; Windows reads `GlobalMemoryStatusEx`;
/// any other platform fails open to 0 (callers treat 0 as "unknown").
fn system_ram_gb() -> u64 {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|bytes| bytes / (1024 * 1024 * 1024))
            .unwrap_or(0)
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        // SAFETY: `status` is a valid, zero-initialized MEMORYSTATUSEX with
        // dwLength set to its own size, exactly as the API requires; the OS
        // only writes into the struct. Non-zero return means success.
        // Rounded, not truncated, to match the macOS branch above and
        // `inference::hardware_tier::detect()`: Windows almost always
        // reports a bit less than the nominal size, so truncating would put
        // a genuine 16 GB machine at 15 and fail this function's per-model
        // min-RAM gate (`ram_ok` in commands/image.rs) when an identical
        // Mac would pass.
        let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
        if ok != 0 {
            (status.ullTotalPhys as f64 / 1024.0 / 1024.0 / 1024.0).round() as u64
        } else {
            0
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        0
    }
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
        // On macOS (sysctl) and Windows (GlobalMemoryStatusEx) this should
        // return something sane; any other platform fails open to 0.
        let gb = system_ram_gb();
        assert!(
            gb > 0,
            "system_ram_gb returned 0 — RAM detection broken on this platform"
        );
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
