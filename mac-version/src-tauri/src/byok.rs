//! BYOK (bring-your-own-key) cloud-provider credential storage. API keys
//! live in an on-disk secret store at `<data_dir>/com.lumizone.fpvdesktop/
//! secrets.json` (0600 on Unix). Plain SQLite would commingle them with
//! app data; the dedicated file keeps them isolated and trivially
//! resettable without an OS-level password prompt.

use crate::error::{AppError, AppResult};
use crate::inference::cloud::CloudProvider;
use crate::secret_store;

const SERVICE: &str = "com.lumizone.fpvdesktop.byok";

pub fn save(provider: CloudProvider, api_key: &str) -> AppResult<()> {
    if api_key.trim().is_empty() {
        return Err(AppError::Config("empty api key".into()));
    }
    secret_store::set(SERVICE, provider.as_str(), api_key.trim())
}

pub fn read(provider: CloudProvider) -> AppResult<Option<String>> {
    secret_store::get(SERVICE, provider.as_str())
}

pub fn delete(provider: CloudProvider) -> AppResult<()> {
    secret_store::delete(SERVICE, provider.as_str())
}

pub fn list_configured() -> AppResult<Vec<CloudProvider>> {
    let mut out = Vec::new();
    for p in CloudProvider::ALL {
        if read(p)?.is_some() {
            out.push(p);
        }
    }
    Ok(out)
}

/// The `Custom` (generic OpenAI-compatible) provider needs a base URL
/// alongside its key. Stored under a dedicated key in the same service
/// namespace so it can't collide with the per-provider API keys.
const CUSTOM_BASE_URL_KEY: &str = "custom_base_url";

pub fn save_base_url(url: &str) -> AppResult<()> {
    if url.trim().is_empty() {
        return Err(AppError::Config("empty base url".into()));
    }
    secret_store::set(SERVICE, CUSTOM_BASE_URL_KEY, url.trim())
}

pub fn read_base_url() -> AppResult<Option<String>> {
    secret_store::get(SERVICE, CUSTOM_BASE_URL_KEY)
}

pub fn delete_base_url() -> AppResult<()> {
    secret_store::delete(SERVICE, CUSTOM_BASE_URL_KEY)
}

/// Whether a custom base URL points at the user's OWN machine or LAN —
/// localhost, loopback, RFC-1918 private ranges (10/8, 192.168/16,
/// 172.16/12), link-local 169.254/16, or an mDNS `.local` hostname.
///
/// Two callers, two reasons:
/// - `byok_save` validation: plain `http://` is only allowed for these
///   (a PUBLIC endpoint must be https — keys travel in headers).
/// - the chat pipeline's Offline-Mode gate: a Custom endpoint on
///   localhost/LAN (LM Studio, vLLM, llama.cpp server) is NOT "cloud" —
///   blocking it under Offline Mode would defeat the mode's purpose
///   (keep traffic off the internet, not off the user's own hardware).
///
/// Pure string inspection — no DNS resolution (a hostname that RESOLVES
/// to a private IP still needs `.local`/literal-IP form here; resolving
/// would make this fallible + slow on the per-turn path).
pub fn is_local_base_url(url: &str) -> bool {
    let parsed = match url::Url::parse(url.trim()) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => parsed,
        _ => return false,
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host_lc = host.to_ascii_lowercase();
    if host_lc == "localhost"
        || host_lc.trim_matches(['[', ']']) == "::1"
        || host_lc.ends_with(".local")
    {
        return true;
    }
    // IPv4 literals: loopback + private + link-local ranges.
    let octets: Vec<u8> = host_lc
        .split('.')
        .map(|p| p.parse::<u8>())
        .collect::<Result<_, _>>()
        .unwrap_or_default();
    if octets.len() == 4 {
        return match (octets[0], octets[1]) {
            (127, _) | (10, _) | (192, 168) | (169, 254) => true,
            (172, b) => (16..=31).contains(&b),
            _ => false,
        };
    }
    false
}

/// Plain HTTP is safe only when it cannot leave this machine. LAN services
/// must use TLS because narration and the user's API key travel together.
pub fn is_loopback_base_url(url: &str) -> bool {
    let parsed = match url::Url::parse(url.trim()) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => parsed,
        _ => return false,
    };
    matches!(
        parsed.host_str().map(|host| host.to_ascii_lowercase()),
        Some(host) if host == "localhost" || host == "127.0.0.1" || host.trim_matches(['[', ']']) == "::1"
    )
}

/// Whether the SAVED custom base URL is a local/LAN endpoint. Blocking
/// (secret-store file read) — call via `spawn_blocking` on hot paths.
/// Missing/unreadable URL reads as NOT local (fail-closed: Offline Mode
/// keeps blocking when we can't prove the endpoint is on-machine).
pub fn custom_endpoint_is_local() -> bool {
    read_base_url()
        .ok()
        .flatten()
        .map(|u| is_local_base_url(&u))
        .unwrap_or(false)
}

/// Whether the saved custom endpoint is on this machine. Local-only stories
/// must not send narration or credentials to another device on the LAN.
pub fn custom_endpoint_is_loopback() -> bool {
    read_base_url()
        .ok()
        .flatten()
        .map(|u| is_loopback_base_url(&u))
        .unwrap_or(false)
}

#[cfg(test)]
mod base_url_tests {
    use super::{is_local_base_url, is_loopback_base_url};

    #[test]
    fn localhost_and_loopback_are_local() {
        for u in [
            "http://localhost:1234/v1",
            "http://127.0.0.1:1234/v1",
            "https://localhost/v1",
            "http://[::1]:8080/v1",
            "http://LOCALHOST:1234/v1",
        ] {
            assert!(is_local_base_url(u), "{u} should be local");
        }
    }

    #[test]
    fn rfc1918_and_mdns_are_local() {
        for u in [
            "http://192.168.1.50:1234/v1",
            "http://10.0.0.7:8000/v1",
            "http://172.16.0.2/v1",
            "http://172.31.255.1:1234/v1",
            "http://169.254.10.10/v1",
            "http://gaming-pc.local:1234/v1",
        ] {
            assert!(is_local_base_url(u), "{u} should be local");
        }
    }

    #[test]
    fn public_hosts_are_not_local() {
        for u in [
            "https://api.together.xyz/v1",
            "http://172.32.0.1/v1", // just past the 172.16-31 block
            "http://172.15.0.1/v1", // just before it
            "http://8.8.8.8/v1",
            "http://11.0.0.1/v1",
            "http://192.169.0.1/v1",
            "http://mylocal.example.com/v1", // ".local" must be a SUFFIX label
            "not-a-url",
            "",
        ] {
            assert!(!is_local_base_url(u), "{u} should NOT be local");
        }
    }

    #[test]
    fn only_loopback_can_use_plain_http() {
        assert!(is_loopback_base_url("http://localhost:1234/v1"));
        assert!(is_loopback_base_url("http://127.0.0.1:1234/v1"));
        assert!(!is_loopback_base_url("http://192.168.1.50:1234/v1"));
    }

    #[test]
    fn lan_hosts_are_not_loopback() {
        assert!(!is_loopback_base_url("https://192.168.1.50:1234/v1"));
        assert!(!is_loopback_base_url("https://gaming-pc.local:1234/v1"));
    }

    #[test]
    fn userinfo_cannot_hide_a_public_host() {
        assert!(!is_local_base_url("http://localhost@evil.example/v1"));
    }
}
