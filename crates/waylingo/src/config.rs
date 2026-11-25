use serde::{Deserialize, Serialize};
use waylingo_parse::LlmConfig;

/// Which map provider this app uses. One provider per app; no mixing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderChoice {
    /// TomTom. `base_url` overrides `https://api.tomtom.com`, for a proxy.
    TomTom { base_url: Option<String> },
    /// HERE. `base_url` routes every HERE service through a proxy
    /// (`{base}/router`, `{base}/geocode`, `{base}/browse`, `{base}/discover`, `{base}/matrix`).
    Here { base_url: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// Most map requests per query, retries included. Default 25.
    pub max_map_calls: u32,
    /// Retries per map request for 429, 5xx and transport failures. Default 2.
    pub max_map_retries: u8,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_map_calls: 25,
            max_map_retries: 2,
        }
    }
}

/// A secret for signing results (see [`crate::TripResult::signature`]). Never
/// printed and never serialized.
#[derive(Clone, PartialEq, Eq)]
pub struct SigningKey(Vec<u8>);

impl SigningKey {
    /// At least 32 random bytes are recommended.
    #[must_use]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SigningKey(<redacted>)")
    }
}

/// Engine settings. Holds no provider or LLM keys: the host adds those to each
/// request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineConfig {
    pub llm: LlmConfig,
    pub provider: ProviderChoice,
    pub limits: Limits,
    /// When set, results are signed and [`crate::Engine::resume`] accepts only
    /// results this key signed. Set it on servers that take results back from
    /// clients; a phone app resuming its own results does not need it.
    #[serde(skip)]
    pub signing_key: Option<SigningKey>,
}

impl EngineConfig {
    /// Default limits and no signing key.
    #[must_use]
    pub fn new(llm: LlmConfig, provider: ProviderChoice) -> Self {
        Self {
            llm,
            provider,
            limits: Limits::default(),
            signing_key: None,
        }
    }
}
