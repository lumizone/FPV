//! Credential storage. macOS uses Keychain; other platforms retain the
//! permission-restricted file fallback until their native credential backend
//! is wired. Existing file-backed secrets are migrated on first access.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
// CloudProvider is only used by the Keychain-backed `clear_all` on macOS;
// the Windows variant clears the DPAPI-encrypted file wholesale.
#[cfg(target_os = "macos")]
use crate::inference::cloud::CloudProvider;

const APP_DIR: &str = "com.lumizone.fpvdesktop";
const FILE_NAME: &str = "secrets.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    entries: HashMap<String, String>,
}

static STATE: Lazy<Mutex<Option<Store>>> = Lazy::new(|| Mutex::new(None));

/// Windows: encrypt a byte buffer with DPAPI (CryptProtectData). The
/// result can only be decrypted by the same Windows user on the same
/// machine — the exact scope the FPV secret store needs (API keys never
/// leave this device; project backups exclude them regardless).
#[cfg(windows)]
fn dpapi_protect(plain: &[u8]) -> AppResult<Vec<u8>> {
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: plain.len() as u32,
        pbData: plain.as_ptr() as *mut u8,
    };
    let mut out_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: in_blob references live bytes for the duration of the call;
    // out_blob is filled by the API and freed via LocalFree below.
    let ok = unsafe {
        CryptProtectData(
            &in_blob,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        )
    };
    if ok == 0 {
        return Err(AppError::Other("Windows DPAPI protect failed".into()));
    }
    // SAFETY: out_blob.pbData points at out_blob.cbData valid bytes owned
    // by the API; we copy them out, then release the allocation.
    let out =
        unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize) }.to_vec();
    unsafe { windows_sys::Win32::Foundation::LocalFree(out_blob.pbData as _) };
    Ok(out)
}

/// Windows: decrypt a DPAPI blob (CryptUnprotectData). A legacy plaintext
/// file is not a valid blob and returns an error — the caller falls back
/// to parsing it as JSON and migrates it on the next write.
#[cfg(windows)]
fn dpapi_unprotect(cipher: &[u8]) -> AppResult<Vec<u8>> {
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: cipher.len() as u32,
        pbData: cipher.as_ptr() as *mut u8,
    };
    let mut out_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: same contract as dpapi_protect.
    let ok = unsafe {
        CryptUnprotectData(
            &in_blob,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        )
    };
    if ok == 0 {
        return Err(AppError::Other("Windows DPAPI unprotect failed".into()));
    }
    let out =
        unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize) }.to_vec();
    unsafe { windows_sys::Win32::Foundation::LocalFree(out_blob.pbData as _) };
    Ok(out)
}

fn data_path() -> AppResult<PathBuf> {
    let base =
        dirs::data_dir().ok_or_else(|| AppError::Other("no data_dir for secret store".into()))?;
    Ok(base.join(APP_DIR).join(FILE_NAME))
}

fn load() -> AppResult<Store> {
    let path = data_path()?;
    if !path.exists() {
        return Ok(Store::default());
    }
    let bytes = fs::read(&path)?;
    if bytes.is_empty() {
        return Ok(Store::default());
    }
    #[cfg(windows)]
    {
        // DPAPI-first: on Windows secrets.json is an encrypted blob. A
        // legacy plaintext file (from before DPAPI) is not a valid blob —
        // fall through to the JSON path below and get re-saved encrypted
        // on the next write (save() always encrypts here).
        if let Ok(plain) = dpapi_unprotect(&bytes) {
            return parse_store_json(&path, &plain);
        }
    }
    parse_store_json(&path, &bytes)
}

/// Parse the plaintext JSON store; corrupt files are quarantined with a
/// timestamp so a bad file bricks no subsequent secret access.
fn parse_store_json(path: &std::path::Path, bytes: &[u8]) -> AppResult<Store> {
    match serde_json::from_slice::<Store>(bytes) {
        Ok(store) => Ok(store),
        Err(err) => {
            // Corrupt or future-version file. Pre-2026-05-22 we
            // propagated the parse error, which made every subsequent
            // `secret_store::{get, set, delete}` call fail with the
            // same JSON error — locking the user out of ALL stored
            // secrets (BYOK keys, Telegram token, trial timestamp)
            // until they manually deleted the file. Safer: quarantine
            // the corrupt file with a timestamp and start clean. New
            // secrets land normally; the user re-enters BYOK / Telegram
            // once instead of being permanently bricked.
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let quarantine = path.with_extension(format!("json.corrupt.{stamp}"));
            // Rename can fail on permission/disk-full. Previously we
            // ignored the result and returned `Store::default()`, which
            // meant the very next `save()` would write over the corrupt
            // file we never quarantined — silently destroying whatever
            // was recoverable (BYOK keys, Telegram token, trial timestamp)
            // before the user noticed. Now: if rename fails, propagate
            // the original parse error so the boot path surfaces "your
            // secrets file is corrupt and could not be moved aside" to
            // the user, instead of pretending everything is fine and
            // overwriting their data.
            if let Err(rename_err) = fs::rename(path, &quarantine) {
                tracing::error!(
                    parse_err = ?err,
                    rename_err = ?rename_err,
                    "secret_store: corrupt JSON and quarantine rename failed; refusing to overwrite",
                );
                return Err(AppError::Other(format!(
                    "secret store at {} is corrupt and could not be moved aside ({}). \
                     Please move or remove the file manually before continuing.",
                    path.display(),
                    rename_err,
                )));
            }
            tracing::error!(
                ?err,
                quarantined = %quarantine.display(),
                "secret_store: corrupt JSON; quarantined and starting clean"
            );
            Ok(Store::default())
        }
    }
}

fn save(store: &Store) -> AppResult<()> {
    let path = data_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        // `create_dir_all` honours the process umask (typically 0022 →
        // 0755). Force 0700 so other local users can't list the dir.
        // The 0600 file inside still hides content either way, but
        // tightening the dir matches the privacy expectation users have
        // when we say "file-backed secrets".
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(parent)?.permissions();
            perms.set_mode(0o700);
            fs::set_permissions(parent, perms)?;
        }
    }
    let tmp = path.with_extension("json.tmp");
    {
        // On Unix, create the tmp file with 0600 from the first byte —
        // previously perms were applied AFTER writing + syncing, so the
        // file existed briefly with the process-umask default (often
        // 0644), exposing secrets to other local users in the gap
        // between create and set_permissions.
        #[cfg(unix)]
        let mut f = {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            // `mode` is ignored when the temp file already exists; repair
            // permissions before writing in that case as well.
            let mut perms = file.metadata()?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&tmp, perms)?;
            file
        };
        #[cfg(not(unix))]
        let mut f = fs::File::create(&tmp)?;
        let bytes = serde_json::to_vec_pretty(store)?;
        #[cfg(windows)]
        let bytes = dpapi_protect(&bytes)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn key(service: &str, name: &str) -> String {
    format!("{service}::{name}")
}

fn with_store<F, R>(f: F) -> AppResult<R>
where
    F: FnOnce(&mut Store) -> AppResult<R>,
{
    let mut guard = STATE
        .lock()
        .map_err(|_| AppError::Other("secret store mutex poisoned".into()))?;
    if guard.is_none() {
        *guard = Some(load()?);
    }
    let store = guard.as_mut().expect("store loaded above");
    f(store)
}

#[cfg(not(target_os = "macos"))]
fn file_set(service: &str, name: &str, value: &str) -> AppResult<()> {
    with_store(|store| {
        // Insert + save, then revert in-memory on save failure. Without
        // this revert, a disk-full / EIO would leave the in-memory map
        // holding the new value while the on-disk file still has the
        // old — the next app launch would silently lose the write.
        let k = key(service, name);
        let prev = store.entries.insert(k.clone(), value.to_string());
        if let Err(e) = save(store) {
            match prev {
                Some(old) => {
                    store.entries.insert(k, old);
                }
                None => {
                    store.entries.remove(&k);
                }
            }
            return Err(e);
        }
        Ok(())
    })
}

fn file_get(service: &str, name: &str) -> AppResult<Option<String>> {
    with_store(|store| Ok(store.entries.get(&key(service, name)).cloned()))
}

fn file_delete(service: &str, name: &str) -> AppResult<()> {
    with_store(|store| {
        let k = key(service, name);
        if let Some(prev) = store.entries.remove(&k) {
            if let Err(e) = save(store) {
                store.entries.insert(k, prev);
                return Err(e);
            }
        }
        Ok(())
    })
}

/// Remove all file-backed credentials during an explicit full-data reset.
fn file_clear_all() -> AppResult<()> {
    let mut guard = STATE
        .lock()
        .map_err(|_| AppError::Other("secret store lock poisoned".into()))?;
    *guard = None;
    let path = data_path()?;
    // Remove the active file, a quarantined corrupt copy, and any interrupted
    // atomic-write temp file. Ignore only a deletion race where the file is
    // already gone; report real I/O failures to the caller.
    let mut candidates = vec![path.clone(), path.with_extension("json.tmp")];
    if let Some(parent) = path.parent() {
        if let Ok(entries) = fs::read_dir(parent) {
            candidates.extend(entries.flatten().filter_map(|entry| {
                let name = entry.file_name();
                let name = name.to_str()?;
                (name.starts_with("secrets.json.corrupt.")).then(|| entry.path())
            }));
        }
    }
    let mut first_error = None;
    for candidate in candidates {
        match fs::remove_file(candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) if first_error.is_none() => first_error = Some(error),
            Err(_) => {}
        }
    }
    if let Some(error) = first_error {
        return Err(error.into());
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod dpapi_tests {
    use super::{dpapi_protect, dpapi_unprotect};

    #[test]
    fn dpapi_round_trip_preserves_bytes() {
        let plain = b"openai::sk-test-12345";
        let cipher = dpapi_protect(plain).unwrap();
        assert_ne!(cipher, plain, "DPAPI must not be a no-op");
        assert_eq!(dpapi_unprotect(&cipher).unwrap(), plain);
    }
}

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "com.lumizone.fpvdesktop";

#[cfg(target_os = "macos")]
fn keychain_account(service: &str, name: &str) -> String {
    format!("{service}::{name}")
}

#[cfg(target_os = "macos")]
fn keychain_set(service: &str, name: &str, value: &str) -> AppResult<()> {
    security_framework::passwords::set_generic_password(
        KEYCHAIN_SERVICE,
        &keychain_account(service, name),
        value.as_bytes(),
    )
    .map_err(|error| AppError::Other(format!("macOS Keychain write failed: {error}")))
}

#[cfg(target_os = "macos")]
fn keychain_get(service: &str, name: &str) -> AppResult<Option<String>> {
    match security_framework::passwords::generic_password(
        security_framework::passwords::PasswordOptions::new_generic_password(
            KEYCHAIN_SERVICE,
            &keychain_account(service, name),
        ),
    ) {
        Ok(bytes) => String::from_utf8(bytes).map(Some).map_err(|error| {
            AppError::Other(format!("macOS Keychain value is not UTF-8: {error}"))
        }),
        Err(error) if error.code() == security_framework_sys::base::errSecItemNotFound => Ok(None),
        Err(error) => Err(AppError::Other(format!(
            "macOS Keychain read failed: {error}"
        ))),
    }
}

#[cfg(target_os = "macos")]
fn keychain_delete(service: &str, name: &str) -> AppResult<()> {
    match security_framework::passwords::delete_generic_password(
        KEYCHAIN_SERVICE,
        &keychain_account(service, name),
    ) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == security_framework_sys::base::errSecItemNotFound => Ok(()),
        Err(error) => Err(AppError::Other(format!(
            "macOS Keychain delete failed: {error}"
        ))),
    }
}

#[cfg(target_os = "macos")]
pub fn set(service: &str, name: &str, value: &str) -> AppResult<()> {
    keychain_set(service, name, value)?;
    // Remove the old plaintext copy only after Keychain confirms the write.
    file_delete(service, name)
}

#[cfg(target_os = "macos")]
pub fn get(service: &str, name: &str) -> AppResult<Option<String>> {
    if let Some(value) = keychain_get(service, name)? {
        return Ok(Some(value));
    }
    // One-time migration for installs created before Keychain support.
    if let Some(value) = file_get(service, name)? {
        keychain_set(service, name, &value)?;
        file_delete(service, name)?;
        return Ok(Some(value));
    }
    Ok(None)
}

#[cfg(target_os = "macos")]
pub fn delete(service: &str, name: &str) -> AppResult<()> {
    keychain_delete(service, name)?;
    file_delete(service, name)
}

#[cfg(target_os = "macos")]
pub fn clear_all() -> AppResult<()> {
    // These are the accounts currently used by BYOK. File entries are also
    // removed so a reset cannot leave a recoverable plaintext copy behind.
    // Iterate the CloudProvider enum (plus the custom-base-url entry) so a
    // newly added provider can never be silently skipped on reset.
    let mut names: Vec<&'static str> =
        CloudProvider::ALL.iter().map(|p| p.as_str()).collect();
    names.push("custom_base_url");
    let mut first_error = None;
    for name in names {
        // Continue past individual failures so one stuck account can't leave
        // the rest (and the plaintext file) uncleaned, but preserve failure
        // semantics for the command caller.
        if let Err(err) = keychain_delete("com.lumizone.fpvdesktop.byok", name) {
            tracing::warn!(?err, %name, "keychain delete failed during clear_all");
            if first_error.is_none() {
                first_error = Some(err);
            }
        }
    }
    if let Err(err) = file_clear_all() {
        if first_error.is_none() {
            first_error = Some(err);
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(not(target_os = "macos"))]
pub fn set(service: &str, name: &str, value: &str) -> AppResult<()> {
    file_set(service, name, value)
}

#[cfg(not(target_os = "macos"))]
pub fn get(service: &str, name: &str) -> AppResult<Option<String>> {
    file_get(service, name)
}

#[cfg(not(target_os = "macos"))]
pub fn delete(service: &str, name: &str) -> AppResult<()> {
    file_delete(service, name)
}

#[cfg(not(target_os = "macos"))]
pub fn clear_all() -> AppResult<()> {
    file_clear_all()
}
