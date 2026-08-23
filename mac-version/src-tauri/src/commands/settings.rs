use crate::db;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use tauri::{AppHandle, Runtime, State};

/// Keys that have a dedicated Tauri command for toggling and MUST NOT be
/// writable from the generic `setting_set` path. Most are privacy gates
/// — flipping them through `setting_set` bypasses the explicit Settings
/// → Privacy toggle the user expects to control them, and DevTools
/// could otherwise re-enable a hidden surface (slap mic detector,
/// proactive calendar access) without ever opening that tab.
///
/// License/trial keys are also blocked: they're written by Polar/Dodo
/// flows after server-side validation, never by user-driven preferences.
const PROTECTED_KEYS: &[&str] = &[
    "slap_enabled",
    "mic_slap_enabled",
    "calendar_proactive_enabled",
    "telegram_token",
    "crypto_salt_hex",
    "hardware_uuid",
    "last_known_version",
];

fn ensure_writable(key: &str) -> AppResult<()> {
    if PROTECTED_KEYS.contains(&key) {
        return Err(AppError::Other(format!(
            "setting `{key}` is read-only via setting_set — use its dedicated command"
        )));
    }
    Ok(())
}

/// Keys whose VALUES are secrets and must NEVER be surfaced to the
/// renderer via the generic getters. Their legitimate data is exposed
/// only through purpose-built commands (e.g. `license_status`) that
/// return derived status, not the raw secret.
const SECRET_READ_DENYLIST: &[&str] = &[
    "telegram_token",
    "crypto_salt_hex",
    "hardware_uuid",
    // Legacy license values may remain in databases created before the
    // license module was removed; never expose them to the renderer.
    "license_key",
    "license_activation_id",
];

/// True when `key` may be read via the generic getters. Denylisted keys
/// plus a defensive catch-all for anything that looks credential-shaped.
fn is_readable_key(key: &str) -> bool {
    if SECRET_READ_DENYLIST.contains(&key) {
        return false;
    }
    let k = key.to_ascii_lowercase();
    !(k.contains("secret") || k.contains("passphrase") || k.contains("private_key"))
}

/// Validate (and where sensible clamp) a value before it's persisted, so a
/// value arriving straight over the IPC boundary — bypassing the UI's
/// bounded `<select>` — can't drive a downstream consumer into a bad
/// state. Returns the value to actually store. Unknown keys pass through
/// unchanged. Boolean prefs need no validation: every consumer reads them
/// as `== "true"`, so any other string is simply treated as false.
fn validate_setting(key: &str, value: &str) -> AppResult<String> {
    match key {
        // Drives Ollama's `num_ctx`; a huge value would try to allocate a
        // giant KV cache and OOM. Clamp to the range the reader honours.
        "contextSize" => {
            let n: i64 = value.trim().parse().map_err(|_| {
                AppError::Other(format!("contextSize must be an integer, got {value:?}"))
            })?;
            Ok(n.clamp(512, 131_072).to_string())
        }
        // Maps to OLLAMA_NUM_GPU at Ollama spawn.
        "device" => {
            let v = value.trim();
            if matches!(v, "auto" | "cpu" | "metal" | "gpu") {
                Ok(v.to_string())
            } else {
                Err(AppError::Other(format!(
                    "device must be one of auto|cpu|metal|gpu, got {value:?}"
                )))
            }
        }
        _ => Ok(value.to_string()),
    }
}

#[tauri::command]
pub async fn setting_get(state: State<'_, AppState>, key: String) -> AppResult<Option<String>> {
    // Secret values are never returned through the generic getter — read
    // them via their dedicated command instead.
    if !is_readable_key(&key) {
        return Ok(None);
    }
    let conn = state.db.lock().await;
    db::meta_get(&conn, &key)
}

#[tauri::command]
pub async fn setting_get_all(
    state: State<'_, AppState>,
) -> AppResult<std::collections::HashMap<String, String>> {
    let conn = state.db.lock().await;
    let mut stmt = conn.prepare("SELECT key, value FROM app_meta")?;
    let mut rows = stmt.query([])?;
    let mut map = std::collections::HashMap::new();
    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        let value: String = row.get(1)?;
        // Never hand secret values to the renderer in the bulk dump.
        if is_readable_key(&key) {
            map.insert(key, value);
        }
    }
    Ok(map)
}

#[tauri::command]
pub async fn setting_set<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> AppResult<()> {
    ensure_writable(&key)?;
    // Validate/clamp before persist so an IPC caller can't bypass the UI's
    // bounded inputs (e.g. contextSize → num_ctx OOM).
    let value = validate_setting(&key, &value)?;
    {
        let conn = state.db.lock().await;
        db::meta_set(&conn, &key, &value)?;
    }
    // Side-effects on specific keys. Keep this list minimal so the
    // generic setter doesn't sprout a per-key match — most prefs are
    // observed by the frontend via `setting_get_all`.
    if key == "language" {
        // Tray menu labels are baked into the macOS NSMenu at build
        // time; re-creating with the new language keeps the tray in
        // sync with the rest of the UI. Errors are non-fatal (the
        // user can re-launch as a last resort).
        if let Err(err) = crate::tray::relabel(&app) {
            tracing::warn!(?err, "tray relabel after language change failed");
        }
    }
    if matches!(key.as_str(), "lowPowerMode" | "semanticMemoryEnabled") {
        // Spawn-time Ollama environment cannot be changed in place. Stop only
        // the FPV-owned daemon; the next local request restarts it with the new
        // energy/memory policy. Never stop a user's system Ollama instance.
        state.sidecars.shutdown_all(&state.ollama).await?;
    }
    Ok(())
}

#[cfg(test)]
mod secret_read_tests {
    use super::is_readable_key;

    #[test]
    fn denylisted_secret_keys_are_not_readable() {
        for k in [
            "crypto_salt_hex",
            "hardware_uuid",
            "license_key",
            "license_activation_id",
            "telegram_token",
        ] {
            assert!(!is_readable_key(k), "{k} must not be readable");
        }
    }

    #[test]
    fn credential_shaped_keys_are_not_readable() {
        assert!(!is_readable_key("some_secret"));
        assert!(!is_readable_key("user_passphrase"));
        assert!(!is_readable_key("x_private_key"));
    }

    #[test]
    fn ordinary_prefs_are_readable() {
        for k in [
            "theme",
            "language",
            "contextSize",
            "offlineMode",
            "autoUpdate",
            // Non-credential license STATE stays readable (just a timestamp).
            "license_lifetime_since",
        ] {
            assert!(is_readable_key(k), "{k} should be readable");
        }
    }
}

#[cfg(test)]
mod validate_tests {
    use super::validate_setting;

    #[test]
    fn context_size_clamps_to_range() {
        assert_eq!(
            validate_setting("contextSize", "999999999").unwrap(),
            "131072"
        );
        assert_eq!(validate_setting("contextSize", "100").unwrap(), "512");
        // In-range UI values pass through.
        assert_eq!(validate_setting("contextSize", "8192").unwrap(), "8192");
        assert_eq!(validate_setting("contextSize", "32768").unwrap(), "32768");
    }

    #[test]
    fn context_size_rejects_non_integer() {
        assert!(validate_setting("contextSize", "huge").is_err());
        assert!(validate_setting("contextSize", "").is_err());
    }

    #[test]
    fn device_whitelist() {
        for ok in ["auto", "cpu", "metal", "gpu"] {
            assert_eq!(validate_setting("device", ok).unwrap(), ok);
        }
        assert!(validate_setting("device", "rm -rf").is_err());
        assert!(validate_setting("device", "").is_err());
    }

    #[test]
    fn unknown_keys_pass_through_unchanged() {
        assert_eq!(validate_setting("theme", "snow").unwrap(), "snow");
        assert_eq!(validate_setting("offlineMode", "true").unwrap(), "true");
        assert_eq!(validate_setting("language", "pl").unwrap(), "pl");
    }
}
