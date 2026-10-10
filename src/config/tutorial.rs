use serde::{Deserialize, Serialize};

/// Tutorial feature settings (the AI coaching layer built on top of the
/// existing history / analysis / bot machinery).
///
/// The LLM endpoint (`[tutorial.llm]`) speaks the OpenAI-compatible
/// chat-completions protocol (D-0007): the same shape serves OpenAI,
/// DeepSeek, GLM, Kimi, ... and a local `llama-server`, so switching
/// provider is a `base_url` change, not a code change.
///
/// Two independent gates guard outbound traffic (see
/// [`TutorialConfig::outbound_allowed`]): `upload_consent` is the privacy
/// opt-in for sending (redacted) game data to a cloud provider, `llm`
/// holds the credentials. Everything defaults empty / off so a fresh
/// install never talks to the network until the user opts in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TutorialConfig {
    /// Master switch for tutorial features. Currently only gates outbound
    /// LLM use together with [`Self::upload_consent`]; local-only
    /// features (e.g. pure-number reviews) stay available when off.
    pub enabled: bool,
    /// Privacy opt-in: game-derived material may leave this machine
    /// (redacted) to be turned into natural-language coaching by a cloud
    /// LLM. Independent of `enabled` so the feature can be on while
    /// uploads stay off (degraded, numbers-only mode).
    pub upload_consent: bool,
    /// Cloud LLM endpoint + credentials. See [`LlmConfig`].
    pub llm: LlmConfig,
    /// In-game coach settings (M2, D-0008).
    pub coach: CoachConfig,
}

/// In-game live coach settings (D-0008). The template layer is local and
/// free; the deep-dive layer additionally requires
/// [`TutorialConfig::outbound_allowed`] before anything leaves the
/// machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoachConfig {
    /// Coach tile master switch. **Off by default**: in-game guidance
    /// must stay out of the way unless asked for (ROADMAP non-goals).
    pub enabled: bool,
    /// Template-layer lines per turn (v1 fixed at 1).
    pub max_per_turn: u32,
    /// Whether the tile shows the deep-dive button. Showing it does not
    /// bypass `upload_consent` — the click errors with a consent hint
    /// instead.
    pub deep_dive_button: bool,
}

/// OpenAI-compatible endpoint credentials for the tutorial LLM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// Base URL of the endpoint, **including** the version segment the
    /// provider expects, e.g. `https://api.openai.com/v1`,
    /// `https://api.deepseek.com/v1`, or `http://127.0.0.1:8080/v1` for
    /// a local `llama-server`. A trailing slash is tolerated;
    /// `/chat/completions` is appended by the provider.
    pub base_url: String,
    /// Bearer API key. May be overridden at runtime by the
    /// `TUTORIAL_LLM_API_KEY` environment variable (env wins, so CI and
    /// portable setups don't have to edit `config.toml`).
    pub api_key: String,
    /// Model id to request, e.g. `deepseek-chat`, `glm-4.7-flash`, ... .
    /// Required: leaving it empty would push a provider-default choice
    /// onto the request path where it fails late and costs a mystery
    /// model; an empty config fails fast in [`Self::is_configured`]
    /// instead.
    pub model: String,
    /// Per-request timeout, in milliseconds. Tutorial generation is an
    /// offline batch job (a finished game's review, not a live turn), so
    /// this can afford far more headroom than the bot's react timeout.
    /// Clamped by [`Self::effective_timeout`]; the raw value is stored so
    /// the UI round-trips exactly what the user typed.
    pub request_timeout_ms: u32,
    /// OpenAI-style reasoning-effort hint (`"low"` / `"medium"` /
    /// `"high"`), forwarded verbatim when set. Reasoning-style models
    /// (GLM-5.x etc.) otherwise think for minutes on tutorial prompts —
    /// wasteful here, because every number the model narrates is already
    /// computed locally and cross-checked on the way back. `"low"` skips
    /// the thinking pass on GLM flash tiers. Absent → not sent (providers
    /// without the field stay untouched).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

/// Default per-request timeout. LLM reviews are batch calls that
/// legitimately take tens of seconds on larger prompts; 60s covers the
/// slow providers while still failing fast enough for the UI to
/// degrade to a numbers-only review instead of hanging.
pub const DEFAULT_REQUEST_TIMEOUT_MS: u32 = 60_000;

/// Bounds the raw [`LlmConfig::request_timeout_ms`] is clamped to before
/// use. Below the floor even a warm provider risks being cut off
/// mid-generation; above the ceiling the caller's degradation path
/// (numbers-only fallback) would rather fire.
pub const REQUEST_TIMEOUT_MIN_MS: u32 = 5_000;
pub const REQUEST_TIMEOUT_MAX_MS: u32 = 300_000;

impl Default for TutorialConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            upload_consent: false,
            llm: LlmConfig::default(),
            coach: CoachConfig::default(),
        }
    }
}

impl Default for CoachConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_per_turn: 1,
            deep_dive_button: true,
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            request_timeout_ms: DEFAULT_REQUEST_TIMEOUT_MS,
            reasoning_effort: None,
        }
    }
}

impl LlmConfig {
    /// The API key actually used: the `TUTORIAL_LLM_API_KEY` environment
    /// variable when set (non-empty), else the configured value. Env wins
    /// so keyless configs (CI, shared machines) can still run.
    pub fn effective_api_key(&self) -> String {
        match std::env::var("TUTORIAL_LLM_API_KEY") {
            Ok(k) if !k.trim().is_empty() => k,
            _ => self.api_key.trim().to_string(),
        }
    }

    /// True only when the endpoint is fully configured (URL + key +
    /// model). Used together with the consent gates to decide whether an
    /// outbound call may be attempted at all.
    pub fn is_configured(&self) -> bool {
        !self.base_url.trim().is_empty()
            && !self.effective_api_key().is_empty()
            && !self.model.trim().is_empty()
    }

    /// The timeout actually applied to one chat-completions request,
    /// clamped to `[REQUEST_TIMEOUT_MIN_MS, REQUEST_TIMEOUT_MAX_MS]`.
    /// Clamping (rather than trusting the stored value) keeps a mistyped
    /// `0` from insta-failing every call and an absurd value from
    /// parking the review pipeline for minutes.
    pub fn effective_timeout(&self) -> std::time::Duration {
        let ms = self
            .request_timeout_ms
            .clamp(REQUEST_TIMEOUT_MIN_MS, REQUEST_TIMEOUT_MAX_MS);
        std::time::Duration::from_millis(u64::from(ms))
    }
}

impl TutorialConfig {
    /// True only when material may leave this machine for the cloud LLM:
    /// the feature is on, the user consented to uploads, and the
    /// endpoint is fully configured. Everything else (store reads,
    /// numbers-only reviews) works regardless.
    pub fn outbound_allowed(&self) -> bool {
        self.enabled && self.upload_consent && self.llm.is_configured()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    /// The nested `[tutorial.llm]` table must survive a TOML round-trip
    /// inside the full `AppConfig` — same rationale as the `[bot.api]`
    /// test in `config::bot` (table-among-scalars ordering is easy to
    /// get wrong). Pure string round-trip, no network: every value is a
    /// placeholder.
    #[test]
    fn tutorial_llm_section_round_trips_through_toml() {
        let mut cfg = AppConfig::default();
        cfg.tutorial.upload_consent = true;
        cfg.tutorial.llm.base_url = "http://127.0.0.1:8080/v1".into();
        cfg.tutorial.llm.api_key = "test-key-not-real".into();
        cfg.tutorial.llm.model = "test-model".into();
        cfg.tutorial.llm.request_timeout_ms = 90_000;

        let body = toml::to_string_pretty(&cfg).unwrap();
        assert!(
            body.contains("[tutorial.llm]"),
            "expected a [tutorial.llm] table in:\n{body}"
        );

        let back: AppConfig = toml::from_str(&body).unwrap();
        assert!(back.tutorial.enabled);
        assert!(back.tutorial.upload_consent);
        assert_eq!(back.tutorial.llm.base_url, "http://127.0.0.1:8080/v1");
        assert_eq!(back.tutorial.llm.api_key, "test-key-not-real");
        assert_eq!(back.tutorial.llm.model, "test-model");
        assert_eq!(back.tutorial.llm.request_timeout_ms, 90_000);
        assert!(back.tutorial.outbound_allowed());
    }

    /// Fresh / legacy configs (no `[tutorial]` section, or one without
    /// credentials) must never allow outbound traffic: enabled defaults
    /// on but consent defaults off, and an empty endpoint fails
    /// `is_configured` regardless.
    #[test]
    fn defaults_and_partial_configs_never_allow_outbound() {
        // Full defaults: consent off.
        let cfg = TutorialConfig::default();
        assert!(cfg.enabled);
        assert!(!cfg.upload_consent);
        assert!(!cfg.outbound_allowed());

        // Legacy file with no [tutorial] section at all.
        let app: AppConfig = toml::from_str("[general]\nfirst_run_completed = true\n").unwrap();
        assert!(!app.tutorial.outbound_allowed());

        // Consent given but endpoint half-configured (no model).
        let mut half = TutorialConfig {
            upload_consent: true,
            ..Default::default()
        };
        half.llm.base_url = "https://api.example.com/v1".into();
        half.llm.api_key = "k".into();
        assert!(!half.llm.is_configured());
        assert!(!half.outbound_allowed());
    }

    /// The timeout is clamped, and a config file predating the field
    /// loads with the default instead of failing to parse.
    #[test]
    fn request_timeout_defaults_and_clamps() {
        assert_eq!(
            LlmConfig::default().effective_timeout(),
            std::time::Duration::from_millis(u64::from(DEFAULT_REQUEST_TIMEOUT_MS))
        );
        let mk = |ms: u32| LlmConfig {
            request_timeout_ms: ms,
            ..Default::default()
        };
        assert_eq!(
            mk(0).effective_timeout(),
            std::time::Duration::from_millis(u64::from(REQUEST_TIMEOUT_MIN_MS))
        );
        assert_eq!(
            mk(600_000).effective_timeout(),
            std::time::Duration::from_millis(u64::from(REQUEST_TIMEOUT_MAX_MS))
        );
        assert_eq!(
            serde_json::from_str::<LlmConfig>("{}")
                .unwrap()
                .request_timeout_ms,
            DEFAULT_REQUEST_TIMEOUT_MS
        );
    }

    /// The `[tutorial.coach]` sub-table round-trips and defaults to
    /// disabled — a fresh install must never pop in-game guidance on.
    #[test]
    fn coach_defaults_off_and_round_trips() {
        let d = CoachConfig::default();
        assert!(!d.enabled, "coach must default to off");
        assert_eq!(d.max_per_turn, 1);
        assert!(d.deep_dive_button);

        let mut cfg = AppConfig::default();
        cfg.tutorial.coach = CoachConfig {
            enabled: true,
            max_per_turn: 2,
            deep_dive_button: false,
        };
        let body = toml::to_string_pretty(&cfg).unwrap();
        assert!(
            body.contains("[tutorial.coach]"),
            "expected a [tutorial.coach] table in:\n{body}"
        );
        let back: AppConfig = toml::from_str(&body).unwrap();
        assert!(back.tutorial.coach.enabled);
        assert_eq!(back.tutorial.coach.max_per_turn, 2);
        assert!(!back.tutorial.coach.deep_dive_button);

        // A legacy file with [tutorial] but no [tutorial.coach] parses
        // with the disabled default.
        let legacy = toml::from_str::<AppConfig>(
            "[general]\nfirst_run_completed = true\n[tutorial]\nenabled = true\n",
        )
        .unwrap();
        assert!(!legacy.tutorial.coach.enabled);
    }
}
