//! fal.ai — synchroniczne generowanie obrazu.
//!
//! Zbudowane jak `image/flux.rs`: POST → URL wyniku → pobranie → walidacja
//! PNG, z tymi samymi limitami co reszta ścieżek obrazowych w FPV.
//!
//! Trzy rzeczy przeniesione z implementacji Local Waifu, bo nie wynikają
//! z dokumentacji fal:
//!   - autoryzacja to `Authorization: Key <token>`, nie Bearer;
//!   - ciało odpowiedzi 401 potrafi odbić nagłówek `Authorization`, więc
//!     każdy błąd musi przejść przez `sanitize_error_body`;
//!   - pobranie obrazka z URL-a musi sprawdzić status PRZED zapisem —
//!     wygasły signed URL zwraca stronę HTML, która bez tego zostałaby
//!     zakodowana w base64 i zwrócona jako udany obraz.
//!
//! Endpointy kolejkowane i wideo są poza tym kontraktem.

use base64::Engine;

use crate::error::{AppError, AppResult};
use crate::image::{png_dimensions, read_response_limited, validate_png_payload};
use crate::image::{ImageRequest, ImageResult};
use crate::inference::cloud::{sanitize_error_body, CloudProvider};

const TIMEOUT_SECS: u64 = 120;
const MAX_PNG_BYTES: usize = 25 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;

/// Pełny URL żądania z tego, co wkleił użytkownik. Akceptuje samo id
/// (`fal-ai/flux-2/flash`) i skopiowany URL — obie formy krążą po stronie fal.
fn endpoint_url(model: &str) -> String {
    let id = model.trim().trim_start_matches("https://fal.run/");
    format!("https://fal.run/{id}")
}

/// Pierwszy URL obrazka z koperty fal. Pusta tablica to błąd, nie sukces.
fn first_image_url(body: &[u8]) -> AppResult<String> {
    #[derive(serde::Deserialize)]
    struct Response {
        images: Option<Vec<FalImage>>,
    }
    #[derive(serde::Deserialize)]
    struct FalImage {
        url: Option<String>,
    }

    let parsed: Response = serde_json::from_slice(body)
        .map_err(|_| AppError::Other("fal response was not valid JSON".into()))?;
    parsed
        .images
        .unwrap_or_default()
        .into_iter()
        .find_map(|image| image.url)
        .ok_or_else(|| AppError::Other("fal returned no image URL".into()))
}

fn looks_like_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

pub async fn generate(req: ImageRequest, model: &str) -> AppResult<ImageResult> {
    // Świadomie BEZ domyślnego endpointu: przypięta nazwa modelu zestarzeje
    // się cicho, a objawem będzie błąd u użytkownika, nie w kompilacji.
    let model = model.trim();
    if model.is_empty() {
        return Err(AppError::Config(
            "paste a fal.ai endpoint id (for example `fal-ai/flux-2/flash`) \
             in Settings → Image models"
                .into(),
        ));
    }

    let api_key = tokio::task::spawn_blocking(move || crate::byok::read(CloudProvider::Fal))
        .await
        .map_err(|_| AppError::Other("fal credential lookup failed".into()))??
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| {
            AppError::Config("configure a fal.ai key before generating images".into())
        })?;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .build()
        .map_err(|_| AppError::Other("fal HTTP client setup failed".into()))?;

    let response = client
        .post(endpoint_url(model))
        .header("Authorization", format!("Key {api_key}"))
        .json(&serde_json::json!({
            "prompt": req.prompt,
            "image_size": "square_hd",
            "num_images": 1,
            "output_format": "png",
        }))
        .send()
        .await
        .map_err(|_| AppError::Other("fal request failed".into()))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = read_response_limited(response, MAX_JSON_BYTES, "fal error body too large")
            .await
            .unwrap_or_default();
        let safe = sanitize_error_body(&String::from_utf8_lossy(&body));
        return Err(AppError::Other(format!("fal image {status}: {safe}")));
    }

    let body = read_response_limited(response, MAX_JSON_BYTES, "fal response exceeded size limit")
        .await?;
    let image_url = first_image_url(&body)?;

    let download = client
        .get(&image_url)
        .send()
        .await
        .map_err(|_| AppError::Other("fal image download failed".into()))?;
    if !download.status().is_success() {
        // Wygasły signed URL albo błąd CDN-u zwraca HTML/JSON, nie piksele.
        let status = download.status();
        let body = read_response_limited(download, MAX_JSON_BYTES, "fal error body too large")
            .await
            .unwrap_or_default();
        let safe = sanitize_error_body(&String::from_utf8_lossy(&body));
        return Err(AppError::Other(format!("fal image download {status}: {safe}")));
    }
    let bytes =
        read_response_limited(download, MAX_PNG_BYTES, "fal image exceeded size limit").await?;

    // Komunikat osobno od `validate_png_payload`: fal ma setki endpointów
    // i część zignoruje `output_format`. Samo "payload is not a valid PNG"
    // brzmi jak zepsuta aplikacja zamiast jak zły wybór endpointu.
    if !looks_like_png(&bytes) {
        return Err(AppError::Config(format!(
            "the fal endpoint `{model}` returned an image FPV cannot read — \
             FPV needs PNG. Pick an endpoint that supports `output_format: png`."
        )));
    }
    validate_png_payload(&bytes, MAX_PNG_BYTES)?;
    let (width, height) = png_dimensions(&bytes).unwrap_or((0, 0));

    Ok(ImageResult {
        image_b64: base64::engine::general_purpose::STANDARD.encode(&bytes),
        provider: "fal".into(),
        model: model.to_string(),
        // fal nie raportuje parametrów generowania. Zera i "unknown" zamiast
        // przepisywania tego, o co poprosiliśmy — seed decyduje o tym, czy
        // scenę da się odtworzyć, więc zmyślony jest gorszy niż żaden.
        seed: 0,
        steps: 0,
        cfg_scale: 0.0,
        width,
        height,
        sampler: "unknown".into(),
        scheduler: None,
        prompt_hash: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::{endpoint_url, first_image_url, looks_like_png};

    #[test]
    fn endpoint_id_and_full_url_resolve_to_the_same_request() {
        // Użytkownik może wkleić samo id albo cały URL ze strony fal.
        assert_eq!(endpoint_url("fal-ai/flux-2/flash"), "https://fal.run/fal-ai/flux-2/flash");
        assert_eq!(endpoint_url("https://fal.run/fal-ai/flux-2/flash"), "https://fal.run/fal-ai/flux-2/flash");
        assert_eq!(endpoint_url("  fal-ai/flux-2/flash  "), "https://fal.run/fal-ai/flux-2/flash");
    }

    #[test]
    fn first_image_url_is_extracted_from_the_fal_envelope() {
        let body = br#"{"images":[{"url":"https://cdn.fal/x.png"},{"url":"https://cdn.fal/y.png"}]}"#;
        assert_eq!(first_image_url(body).unwrap(), "https://cdn.fal/x.png");
    }

    #[test]
    fn an_empty_images_array_is_an_error_not_a_silent_success() {
        assert!(first_image_url(br#"{"images":[]}"#).is_err());
        assert!(first_image_url(br#"{"detail":"nope"}"#).is_err());
    }

    #[tokio::test]
    async fn an_empty_model_fails_with_an_instruction_before_any_request() {
        // Musi paść na pustym modelu ZANIM dotknie sieci albo Keychaina —
        // to jest cała treść decyzji "bez domyślnego endpointu".
        let req = super::super::ImageRequest {
            prompt: "a lantern".into(),
            style: super::super::ImageStyle::Raw,
            ..Default::default()
        };
        let err = super::generate(req, "   ").await.unwrap_err().to_string();
        assert!(err.contains("fal-ai/flux-2/flash"), "brak przykladu w komunikacie: {err}");
    }

    #[test]
    fn a_401_body_echoing_the_key_is_redacted_before_it_reaches_the_ui() {
        // fal potrafi odbic naglowek Authorization w tresci bledu. To jest
        // powod, dla ktorego kazda sciezka bledu tutaj idzie przez
        // sanitize_error_body, a nie prosto do AppError.
        let body = r#"{"detail":"Unauthorized","sent":"Authorization: Key sk-fal-secret-123"}"#;
        let safe = crate::inference::cloud::sanitize_error_body(body);
        assert!(!safe.contains("sk-fal-secret-123"), "klucz wyciekl: {safe}");
        assert!(safe.contains("[redacted]"));
    }

    #[test]
    fn looks_like_png_rejects_other_formats() {
        assert!(looks_like_png(b"\x89PNG\r\n\x1a\n rest"));
        assert!(!looks_like_png(b"\xff\xd8\xff\xe0 jpeg"));
        assert!(!looks_like_png(b"RIFF....WEBP"));
        assert!(!looks_like_png(b""));
    }
}
