use std::process::Command;

use serde::Serialize;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Minimal,
    Light,
    Balanced,
    Premium,
    Power,
}

impl Tier {
    pub fn from_ram_gb(ram_gb: u64) -> Self {
        match ram_gb {
            // gemma4:e4b (Light's default) is 9.6 GB on disk. An 8-12 GB
            // machine can't hold that plus the KV cache and the OS, and
            // Tier used to map every value below 8 GB to Light too — a
            // 128 GB probe reading 0 from a failed sysctl call landed on
            // the smallest model regardless. Minimal's gemma4:e2b (7.2 GB)
            // fits the low end without that failure mode.
            0..=12 => Tier::Minimal,
            13..=18 => Tier::Light,
            19..=34 => Tier::Balanced,
            // qwen3.6:35b (Power's default) needs 64 GB min per the
            // catalog; a 57-63 GB machine can't run it. Extending
            // Premium's ceiling to 63 keeps those machines on
            // qwen3.6:27b (needs only 32 GB) instead of a Power
            // recommendation their RAM doesn't meet.
            35..=63 => Tier::Premium,
            _ => Tier::Power,
        }
    }

    // Tags last verified: **2026-07-31** against the Ollama registry
    // (every id in `catalog.rs` + these five returned 200, sizes
    // recomputed from each manifest's layer sizes). Re-verify with a
    // HEAD against the manifest rather than by reading a blog — hard
    // rule 18 exists because model tags drift silently:
    //
    //   curl -s -o /dev/null -w "%{http_code}" \
    //     https://registry.ollama.ai/v2/library/<name>/manifests/<tag>
    //
    // Also checked that day: there is NO mid-size qwen3.6. The family
    // ships 27b and 35b only (4b/8b/9b/12b/14b all 404), so Balanced
    // staying on qwen3.5:9b is the correct choice, not a leftover —
    // don't "fix" it to a 3.6 tag that doesn't exist.
    pub fn default_chat_model(self) -> &'static str {
        match self {
            Tier::Minimal => "gemma4:e2b",  // 7.2 GB on disk, fits 0-12 GB RAM
            Tier::Light => "gemma4:e4b", // 9.6 GB on disk (WebFetch-verified against ollama.com/library), fits 13-18 GB RAM (v0.1.9 swap from qwen3.5:4b)
            Tier::Balanced => "qwen3.5:9b", // 6.6 GB, fits 19-34 GB RAM (no 3.6 at this size — see above)
            Tier::Premium => "qwen3.6:27b", // 17 GB, fits 35-63 GB RAM (ceiling per from_ram_gb, not 56)
            Tier::Power => "qwen3.6:35b",   // 24 GB, 64+ GB RAM (needs 64 min — see from_ram_gb)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HardwareInfo {
    pub chip: String,
    pub ram_gb: u64,
    pub tier: Tier,
    pub recommended_chat_model: String,
}

pub fn detect() -> AppResult<HardwareInfo> {
    let chip = sysctl_str("machdep.cpu.brand_string").unwrap_or_else(|_| "unknown".into());
    let ram_bytes: u64 = sysctl_str("hw.memsize")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let ram_gb = (ram_bytes as f64 / 1024.0 / 1024.0 / 1024.0).round() as u64;
    let tier = Tier::from_ram_gb(ram_gb);
    Ok(HardwareInfo {
        chip,
        ram_gb,
        tier,
        recommended_chat_model: tier.default_chat_model().to_string(),
    })
}

fn sysctl_str(key: &str) -> AppResult<String> {
    // Absolute path: hardware detection runs at boot, where the process
    // may have a stripped PATH (launchd autostart). `/usr/sbin` isn't
    // guaranteed on PATH there; the absolute path is stable across macOS
    // releases.
    let out = Command::new("/usr/sbin/sysctl")
        .args(["-n", key])
        .output()
        .map_err(|e| AppError::Other(format!("sysctl {key}: {e}")))?;
    if !out.status.success() {
        return Err(AppError::Other(format!(
            "sysctl {key} exited {:?}",
            out.status.code()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
