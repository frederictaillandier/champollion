//! ElevenLabs' text-to-speech API, which speaks the cards' words.

use std::time::Duration;

use serde::Serialize;

pub struct ElevenLabs {
    pub url: String,
    pub api_key: String,
    pub voice: String,
    pub model: String,
    agent: ureq::Agent,
}

/// Why a word could not be spoken.
#[derive(Debug)]
pub enum SpeakError {
    /// ElevenLabs refused this text: the next words may work.
    Text(String),
    /// Anything else (network, key, quota): no word will work for a while.
    Service(String),
}

impl std::fmt::Display for SpeakError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Text(e) | Self::Service(e) => f.write_str(e),
        }
    }
}

#[derive(Serialize)]
struct Request<'a> {
    text: &'a str,
    model_id: &'a str,
    /// Without it, a short word can be read in the wrong language (`most`
    /// in English).
    #[serde(skip_serializing_if = "Option::is_none")]
    language_code: Option<&'a str>,
}

impl ElevenLabs {
    pub fn new(url: String, api_key: String, voice: String, model: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Self {
            url: url.trim_end_matches('/').to_owned(),
            api_key,
            voice,
            model,
            agent,
        }
    }

    /// The MP3 of `text` spoken in `lang`, a Tesseract language code.
    pub fn speak(&self, text: &str, lang: &str) -> Result<Vec<u8>, SpeakError> {
        let url = format!(
            "{}/v1/text-to-speech/{}?output_format=mp3_44100_64",
            self.url, self.voice
        );
        let request = Request {
            text,
            model_id: &self.model,
            language_code: iso_639_1(lang),
        };
        self.agent
            .post(&url)
            .header("xi-api-key", &self.api_key)
            .send_json(&request)
            .and_then(|mut r| r.body_mut().read_to_vec())
            .map_err(|e| match e {
                // 401 bad key, 402 and 429 out of quota or credits.
                ureq::Error::StatusCode(code @ (400 | 404 | 422)) => {
                    SpeakError::Text(format!("ElevenLabs answered {code}"))
                }
                e => SpeakError::Service(e.to_string()),
            })
    }
}

/// The ISO 639-1 code ElevenLabs takes, of a Tesseract language code.
fn iso_639_1(tesseract: &str) -> Option<&'static str> {
    Some(match tesseract {
        "ces" => "cs",
        "slk" => "sk",
        "pol" => "pl",
        "deu" => "de",
        "fra" => "fr",
        "spa" => "es",
        "ita" => "it",
        "por" => "pt",
        "nld" => "nl",
        "rus" => "ru",
        "ukr" => "uk",
        "jpn" => "ja",
        "kor" => "ko",
        "chi_sim" | "chi_tra" => "zh",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_tesseract_languages() {
        assert_eq!(iso_639_1("ces"), Some("cs"));
        assert_eq!(iso_639_1("chi_sim"), Some("zh"));
        assert_eq!(iso_639_1("eng+ces"), None);
    }
}
