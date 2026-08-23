//! System health probe used by the frontend to gate the chat UI on
//! Ollama availability. Without this the user can type a message before
//! the sidecar finishes booting and watch the first message fail.

use serde::Serialize;
use tauri::State;
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(windows)]
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

use crate::error::AppResult;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct SystemReady {
    pub ollama_ready: bool,
}

#[tauri::command]
pub async fn system_ready_get(state: State<'_, AppState>) -> AppResult<SystemReady> {
    let ollama_ready = state.ollama.ping().await.unwrap_or(false);
    Ok(SystemReady { ollama_ready })
}

#[derive(Debug, Serialize)]
pub struct SystemEnergy {
    pub on_ac_power: bool,
    pub battery_percent: Option<u8>,
    pub thermally_constrained: bool,
}

#[cfg(target_os = "macos")]
fn parse_battery_status(text: &str) -> (bool, Option<u8>) {
    let on_ac_power = text.contains("AC Power");
    let battery_percent = text
        .split_whitespace()
        .find_map(|part| part.trim_end_matches([';', '%']).parse::<u8>().ok());
    (on_ac_power, battery_percent)
}

#[cfg(target_os = "macos")]
fn parse_thermal_status(text: &str) -> bool {
    text.lines().any(|line| {
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        matches!(key.trim(), "CPU_Speed_Limit" | "GPU_Speed_Limit")
            && value.trim().parse::<u8>().is_ok_and(|limit| limit < 90)
    })
}

#[tauri::command]
pub async fn system_energy_get() -> AppResult<SystemEnergy> {
    #[cfg(target_os = "macos")]
    {
        let battery = Command::new("/usr/bin/pmset").args(["-g", "batt"]).output();
        let thermal = Command::new("/usr/bin/pmset")
            .args(["-g", "therm"])
            .output();
        let battery_text = battery
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
        let thermal_text = thermal
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
        let (on_ac_power, battery_percent) = parse_battery_status(&battery_text);
        Ok(SystemEnergy {
            on_ac_power,
            battery_percent,
            thermally_constrained: parse_thermal_status(&thermal_text),
        })
    }

    #[cfg(windows)]
    {
        // GetSystemPowerStatus is a stable user-mode Windows API and avoids
        // spawning PowerShell/WMI on every health poll. A value of 255 means
        // the battery percentage is unknown (desktop machines commonly report
        // this), so do not expose it as a fabricated 0 or 100 percent.
        let mut status = SYSTEM_POWER_STATUS {
            ACLineStatus: 255,
            BatteryFlag: 255,
            BatteryLifePercent: 255,
            SystemStatusFlag: 0,
            BatteryLifeTime: u32::MAX,
            BatteryFullLifeTime: u32::MAX,
        };
        let ok = unsafe { GetSystemPowerStatus(&mut status) } != 0;
        if !ok {
            // Fail open: inability to query power must never block narration
            // or image generation on a Windows desktop/VM.
            return Ok(SystemEnergy {
                on_ac_power: true,
                battery_percent: None,
                thermally_constrained: false,
            });
        }
        Ok(SystemEnergy {
            on_ac_power: status.ACLineStatus != 0,
            battery_percent: (status.BatteryLifePercent <= 100)
                .then_some(status.BatteryLifePercent),
            // Windows has no supported, portable equivalent of macOS pmset's
            // speed-limit signal. Keep this conservative and fail open rather
            // than claiming thermal throttling from an unreliable heuristic.
            thermally_constrained: false,
        })
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    Ok(SystemEnergy {
        on_ac_power: true,
        battery_percent: None,
        thermally_constrained: false,
    })
}

#[cfg(all(test, target_os = "macos"))]
mod energy_tests {
    use super::{parse_battery_status, parse_thermal_status};

    #[test]
    fn parses_pmset_battery_output() {
        let (ac, percent) =
            parse_battery_status("Now drawing from 'AC Power'\n -InternalBattery-0 83%; charging;");
        assert!(ac);
        assert_eq!(percent, Some(83));
    }

    #[test]
    fn detects_thermal_throttling() {
        assert!(parse_thermal_status(
            "CPU_Speed_Limit = 70\nGPU_Speed_Limit = 100"
        ));
        assert!(!parse_thermal_status("CPU_Speed_Limit = 100"));
    }
}
