//! How an app configures Waylingo: one map provider, one LLM, search settings.
//! Keys are not here: they are constructor arguments, so they never appear in a
//! record's generated `toString()` or `repr()`.

use std::time::Duration;

use waylingo::{Defaults, EngineConfig, Limits, LlmConfig, LlmMode, PromptStyle, ProviderChoice};

use crate::WaylingoError;

/// The map provider the app uses. One provider per app; no mixing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MapProvider {
    Tomtom,
    Here,
}

/// Which LLM reads the request text. It only ever sees the text, the locale and the
/// names of saved places; map data never reaches it.
///
/// `base_url` sends the LLM calls to your own server or proxy instead of the
/// provider (for example, to keep the provider key off end users' devices). It
/// must speak the provider's OpenAI-compatible API.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum LlmBackend {
    /// Anthropic's Claude. `model` defaults to [`DEFAULT_ANTHROPIC_MODEL`].
    Anthropic {
        model: Option<String>,
        base_url: Option<String>,
    },
    OpenAi {
        model: String,
        base_url: Option<String>,
    },
    /// Google Gemini through its OpenAI-compatible endpoint.
    Gemini {
        model: String,
        base_url: Option<String>,
    },
    /// Any OpenAI-compatible server, for example a local `mlx_lm.server`, vLLM or
    /// llama.cpp server. Set `fine_tuned` when it serves Waylingo's fine-tuned
    /// parser model (it then gets the short prompt it was trained with).
    OpenAiCompatible {
        base_url: String,
        model: String,
        fine_tuned: bool,
    },
}

impl LlmBackend {
    /// True when calls go to the provider itself, which needs the provider's key.
    pub(crate) fn needs_key(&self) -> bool {
        match self {
            Self::Anthropic { base_url, .. }
            | Self::OpenAi { base_url, .. }
            | Self::Gemini { base_url, .. } => base_url.is_none(),
            Self::OpenAiCompatible { .. } => false,
        }
    }
}

/// Developer settings for searching. Unset fields keep Waylingo's defaults; a value
/// the end user says in the request ("within 5 minutes") wins for that request.
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct SearchSettings {
    /// Prefer places within this distance of each anchor point, metres. Default 5 000.
    #[uniffi(default = None)]
    pub anchor_radius_m: Option<u32>,
    /// Route searched on each side of an anchor, metres. Default 3 x radius.
    #[uniffi(default = None)]
    pub search_span_m: Option<u32>,
    /// Results asked per search (capped by the provider). Default 20.
    #[uniffi(default = None)]
    pub search_limit: Option<u8>,
    /// Largest detour accepted, seconds. Default 600.
    #[uniffi(default = None)]
    pub max_detour_s: Option<u32>,
    /// Options returned per stop (the pick plus alternatives), 1 to 5. Default 3.
    #[uniffi(default = None)]
    pub top_n: Option<u8>,
    /// Never use a place beyond the radius. Default false.
    #[uniffi(default = None)]
    pub strict_radius: Option<bool>,
}

impl SearchSettings {
    pub(crate) fn defaults(&self) -> Defaults {
        let d = Defaults::default();
        Defaults {
            anchor_radius_m: self.anchor_radius_m.unwrap_or(d.anchor_radius_m),
            search_span_m: self.search_span_m.or(d.search_span_m),
            search_limit: self.search_limit.unwrap_or(d.search_limit),
            max_detour_s: self.max_detour_s.unwrap_or(d.max_detour_s),
            top_n: self.top_n.unwrap_or(d.top_n),
            strict_radius: self.strict_radius.unwrap_or(d.strict_radius),
            every_window_pct: d.every_window_pct,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WaylingoConfig {
    pub map_provider: MapProvider,
    pub llm: LlmBackend,
    /// Send map requests to your own proxy instead of the provider's host. The map
    /// key may then be empty, or carry your proxy's own token.
    #[uniffi(default = None)]
    pub map_base_url: Option<String>,
    #[uniffi(default = None)]
    pub settings: Option<SearchSettings>,
    /// Most map requests per query, retries included. Default 25.
    #[uniffi(default = None)]
    pub max_map_calls: Option<u32>,
    /// Map requests per second across every query on this instance. Default 4.5
    /// (under TomTom's default 5 per second); raise it to match your plan.
    #[uniffi(default = None)]
    pub map_requests_per_second: Option<f64>,
    /// Longest a whole query may take, milliseconds, including waiting for map
    /// request slots. Default 60 000; at least 1 000.
    #[uniffi(default = None)]
    pub timeout_ms: Option<u32>,
}

/// The model used when [`LlmBackend::Anthropic`] gives none. Set `model` to pin
/// another one; a default baked into an app may be retired by the provider.
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-5-5";
/// Default map pacing: just under TomTom's default 5 requests per second.
const DEFAULT_MAP_RPS: f64 = 4.5;
/// Default and smallest whole-query time limit.
const DEFAULT_TIMEOUT_MS: u32 = 60_000;
const MIN_TIMEOUT_MS: u32 = 1_000;

fn http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

impl WaylingoConfig {
    pub(crate) fn engine_config(&self) -> Result<EngineConfig, WaylingoError> {
        let bad = |message: &str| WaylingoError::InvalidConfig {
            message: message.into(),
        };
        let pick = |own: &Option<String>, default: &str| -> Result<String, WaylingoError> {
            match own {
                Some(u) if !http_url(u) => Err(bad("base_url must start with http:// or https://")),
                Some(u) => Ok(u.clone()),
                None => Ok(default.to_string()),
            }
        };
        let llm = match &self.llm {
            LlmBackend::Anthropic { model, base_url } => LlmConfig::new(
                pick(base_url, "https://api.anthropic.com/v1")?,
                model
                    .clone()
                    .unwrap_or_else(|| DEFAULT_ANTHROPIC_MODEL.into()),
                LlmMode::ToolCall,
            ),
            LlmBackend::OpenAi { model, base_url } => LlmConfig::new(
                pick(base_url, "https://api.openai.com/v1")?,
                model,
                LlmMode::JsonSchema,
            ),
            LlmBackend::Gemini { model, base_url } => LlmConfig::new(
                pick(
                    base_url,
                    "https://generativelanguage.googleapis.com/v1beta/openai",
                )?,
                model,
                LlmMode::JsonSchema,
            ),
            LlmBackend::OpenAiCompatible {
                base_url,
                model,
                fine_tuned,
            } => {
                if !http_url(base_url) {
                    return Err(bad("base_url must start with http:// or https://"));
                }
                let mut c = LlmConfig::new(base_url, model, LlmMode::PromptJson);
                if *fine_tuned {
                    c.prompt = PromptStyle::Compact;
                }
                c
            }
        };
        if llm.model.trim().is_empty() {
            return Err(bad("the LLM model name is empty"));
        }
        if let Some(u) = &self.map_base_url
            && !http_url(u)
        {
            return Err(bad("map_base_url must start with http:// or https://"));
        }
        let base_url = self.map_base_url.clone();
        let provider = match self.map_provider {
            MapProvider::Tomtom => ProviderChoice::TomTom { base_url },
            MapProvider::Here => ProviderChoice::Here { base_url },
        };
        let mut cfg = EngineConfig::new(llm, provider);
        cfg.limits = Limits {
            max_map_calls: self
                .max_map_calls
                .unwrap_or(Limits::default().max_map_calls),
            ..Limits::default()
        };
        if cfg.limits.max_map_calls < 3 {
            return Err(bad("max_map_calls must be at least 3"));
        }
        Ok(cfg)
    }

    pub(crate) fn defaults(&self) -> Defaults {
        self.settings.clone().unwrap_or_default().defaults()
    }

    /// Least time between map request starts.
    pub(crate) fn map_spacing(&self) -> Result<Duration, WaylingoError> {
        let rps = self.map_requests_per_second.unwrap_or(DEFAULT_MAP_RPS);
        if !(rps.is_finite() && rps > 0.0 && rps <= 1_000.0) {
            return Err(WaylingoError::InvalidConfig {
                message: "map_requests_per_second must be between 0 and 1000".into(),
            });
        }
        Ok(Duration::from_secs_f64(1.0 / rps))
    }

    pub(crate) fn deadline(&self) -> Result<Duration, WaylingoError> {
        let ms = self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        if ms < MIN_TIMEOUT_MS {
            return Err(WaylingoError::InvalidConfig {
                message: format!("timeout_ms must be at least {MIN_TIMEOUT_MS}"),
            });
        }
        Ok(Duration::from_millis(u64::from(ms)))
    }

    /// Check the keys against the configuration: a provider without a proxy needs
    /// its key.
    pub(crate) fn check_keys(
        &self,
        map_key: &str,
        llm_key: Option<&str>,
    ) -> Result<(), WaylingoError> {
        let blank = |k: Option<&str>| k.is_none_or(|k| k.trim().is_empty());
        if self.map_base_url.is_none() && blank(Some(map_key)) {
            return Err(WaylingoError::InvalidConfig {
                message: "map_key is empty (it may be empty only with map_base_url)".into(),
            });
        }
        if self.llm.needs_key() && blank(llm_key) {
            return Err(WaylingoError::InvalidConfig {
                message: "llm_key is required for this LLM backend unless base_url is set".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(llm: LlmBackend) -> WaylingoConfig {
        WaylingoConfig {
            map_provider: MapProvider::Tomtom,
            llm,
            map_base_url: None,
            settings: None,
            max_map_calls: None,
            map_requests_per_second: None,
            timeout_ms: None,
        }
    }

    fn anthropic() -> LlmBackend {
        LlmBackend::Anthropic {
            model: None,
            base_url: None,
        }
    }

    #[test]
    fn backends_map_to_endpoints_and_modes() {
        let c = cfg(anthropic()).engine_config().unwrap();
        assert_eq!(c.llm.base_url, "https://api.anthropic.com/v1");
        assert_eq!(c.llm.model, DEFAULT_ANTHROPIC_MODEL);
        assert_eq!(c.llm.mode, LlmMode::ToolCall);

        let proxied = cfg(LlmBackend::OpenAi {
            model: "m".into(),
            base_url: Some("https://llm.myapp.example/v1".into()),
        })
        .engine_config()
        .unwrap();
        assert_eq!(proxied.llm.base_url, "https://llm.myapp.example/v1");
        assert_eq!(proxied.llm.mode, LlmMode::JsonSchema);

        let c = cfg(LlmBackend::OpenAiCompatible {
            base_url: "http://127.0.0.1:8765/v1".into(),
            model: "default".into(),
            fine_tuned: true,
        })
        .engine_config()
        .unwrap();
        assert_eq!(c.llm.mode, LlmMode::PromptJson);
        assert_eq!(c.llm.prompt, PromptStyle::Compact);
    }

    #[test]
    fn bad_configs_are_rejected() {
        let empty = cfg(LlmBackend::OpenAi {
            model: " ".into(),
            base_url: None,
        });
        assert!(empty.engine_config().is_err());
        let no_scheme = cfg(LlmBackend::OpenAiCompatible {
            base_url: "localhost:8765".into(),
            model: "m".into(),
            fine_tuned: false,
        });
        assert!(no_scheme.engine_config().is_err());
        let mut few = cfg(anthropic());
        few.max_map_calls = Some(1);
        assert!(few.engine_config().is_err());
        let mut instant = cfg(anthropic());
        instant.timeout_ms = Some(0);
        assert!(instant.deadline().is_err());
        let mut stalled = cfg(anthropic());
        stalled.map_requests_per_second = Some(0.0);
        assert!(stalled.map_spacing().is_err());
    }

    #[test]
    fn keys_are_required_unless_a_proxy_is_used() {
        let direct = cfg(anthropic());
        assert!(direct.check_keys("map", Some("llm")).is_ok());
        assert!(
            direct.check_keys("", Some("llm")).is_err(),
            "no map key, no proxy"
        );
        assert!(
            direct.check_keys("map", None).is_err(),
            "no LLM key, no proxy"
        );

        let mut proxied = cfg(LlmBackend::Anthropic {
            model: None,
            base_url: Some("https://llm.myapp.example/v1".into()),
        });
        proxied.map_base_url = Some("https://maps.myapp.example".into());
        assert!(proxied.check_keys("", None).is_ok());
    }

    #[test]
    fn pacing_follows_the_setting() {
        let mut c = cfg(anthropic());
        assert_eq!(c.map_spacing().unwrap(), Duration::from_secs_f64(1.0 / 4.5));
        c.map_requests_per_second = Some(50.0);
        assert_eq!(c.map_spacing().unwrap(), Duration::from_millis(20));
    }

    #[test]
    fn unset_settings_keep_the_defaults() {
        let d = SearchSettings {
            anchor_radius_m: Some(2_000),
            ..SearchSettings::default()
        }
        .defaults();
        assert_eq!(d.anchor_radius_m, 2_000);
        assert_eq!(d.top_n, Defaults::default().top_n);
    }
}
