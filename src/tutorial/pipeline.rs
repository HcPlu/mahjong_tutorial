//! Review generation pipeline (M1): the fixed call order the module
//! README prescribes for IPC, owned here so commands stay thin.
//!
//! Store cache → consent gate → assemble → redact → prompt → provider →
//! parse (anti-hallucination) → stamp → save. Any failure is an error the
//! caller degrades on (numbers-only review) — never retried here.

use anyhow::{bail, Context, Result};

use crate::config::TutorialConfig;
use crate::schema::MjaiEvent;
use crate::tutorial::assemble::build_review_material;
use crate::tutorial::download::parse_review_doc;
use crate::tutorial::material::{ReviewCoverage, ReviewDoc, ReviewMaterial};
use crate::tutorial::provider::{ChatRequest, OpenAiCompatProvider, TutorialProvider};
use crate::tutorial::store::TutorialStore;
use crate::tutorial::upload;

/// Turns per chat request. Caps output length per call so the JSON
/// contract survives providers with modest completion limits; `All`
/// coverage on a long game simply runs more chunks.
const TURNS_PER_REQUEST: usize = 15;
const TEMPERATURE: f32 = 0.2;
/// Reasoning-style models (GLM-4.7 etc.) spend invisible thinking tokens
/// from the same budget before writing the JSON — measured against the
/// real endpoint, 4000 starves the content to empty on a 6-turn prompt.
const MAX_TOKENS_PER_REQUEST: u32 = 8000;

#[derive(Debug)]
pub struct GenerateOutcome {
    pub doc: ReviewDoc,
    /// True when the cached review was returned without a network call.
    pub cached: bool,
}

/// Generate (or return the cached) review for one recorded game.
///
/// `events` / `seat` / `player_name` come from the history store; the
/// caller is `tutorial_generate_review` in `src/ipc/commands.rs`.
pub async fn generate_review(
    cfg: &TutorialConfig,
    store: &TutorialStore,
    events: &[MjaiEvent],
    seat: u8,
    game_id: &str,
    player_name: Option<String>,
    coverage: ReviewCoverage,
) -> Result<GenerateOutcome> {
    // Cache before request (invariant 3): one LLM spend per game.
    if let Some(doc) = store.load_review(game_id)? {
        return Ok(GenerateOutcome { doc, cached: true });
    }

    // Consent before call (invariant 2).
    if !upload::outbound_allowed(cfg) {
        bail!(
            "outbound not allowed: check that the tutorial is enabled, upload consent is \
             given, and the LLM endpoint (base_url / api_key / model) is configured"
        );
    }

    let material = build_review_material(events, seat, game_id, player_name, coverage)
        .context("failed to assemble review material from the recorded event stream")?;
    if material.turns.is_empty() {
        bail!("no reviewable decision points for seat {seat} in this game");
    }

    // Redact once, then chunk the redacted copy — every chunk's parse is
    // validated against exactly what left the machine.
    let redacted = upload::redact(&material);
    let provider = OpenAiCompatProvider::new(&cfg.llm)
        .context("failed to build the tutorial LLM provider from config")?;

    let mut turns = Vec::with_capacity(redacted.turns.len());
    let mut overview: Option<String> = None;
    let mut model: Option<String> = None;
    for chunk in redacted.turns.chunks(TURNS_PER_REQUEST) {
        let sub = ReviewMaterial {
            game_id: redacted.game_id.clone(),
            seat: redacted.seat,
            num_players: redacted.num_players,
            player_name: None,
            turns: chunk.to_vec(),
        };
        let resp = provider
            .complete(ChatRequest {
                messages: upload::build_messages(&sub),
                temperature: Some(TEMPERATURE),
                max_tokens: Some(MAX_TOKENS_PER_REQUEST),
            })
            .await
            .context("LLM request failed — degrading to a numbers-only review")?;
        let doc = parse_review_doc(&resp.content, &sub)
            .context("LLM reply rejected by validation — degrading to a numbers-only review")?;
        if overview.is_none() {
            overview = doc.overview.clone();
        }
        if model.is_none() {
            model = resp.model.clone();
        }
        turns.extend(doc.turns);
    }

    let doc = ReviewDoc {
        game_id: redacted.game_id.clone(),
        seat: redacted.seat,
        model,
        created_at: chrono::Utc::now().to_rfc3339(),
        overview,
        turns,
    };
    store.save_review(&doc)?;
    Ok(GenerateOutcome { doc, cached: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LlmConfig;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!(
            "akagi-tutorial-pipeline-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn cfg_with_consent() -> TutorialConfig {
        TutorialConfig {
            upload_consent: true,
            llm: LlmConfig {
                base_url: "http://127.0.0.1:9/v1".into(),
                api_key: "k".into(),
                model: "m".into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn consent_gate_rejects_before_any_network() {
        let root = temp_root("gate");
        let store = TutorialStore::new(root.clone()).unwrap();
        let cfg = TutorialConfig::default(); // consent off
                                             // Empty events: assembly succeeds with zero turns but the gate
                                             // fires first, so the error names consent, not emptiness.
        let err = futures_lite_block_on(generate_review(
            &cfg,
            &store,
            &[],
            0,
            "01JPIPELINETEST",
            None,
            ReviewCoverage::All,
        ))
        .unwrap_err();
        assert!(format!("{err:#}").contains("outbound not allowed"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn cached_doc_short_circuits() {
        let root = temp_root("cache");
        let store = TutorialStore::new(root.clone()).unwrap();
        // Seed the cache directly through the save zone.
        let mut doc = ReviewDoc {
            game_id: "01JPIPELINETEST".into(),
            seat: 0,
            ..Default::default()
        };
        doc.overview = Some("cached".into());
        store.save_review(&doc).unwrap();

        let out = futures_lite_block_on(generate_review(
            &cfg_with_consent(),
            &store,
            &[],
            0,
            "01JPIPELINETEST",
            None,
            ReviewCoverage::All,
        ))
        .unwrap();
        assert!(out.cached);
        assert_eq!(out.doc.overview.as_deref(), Some("cached"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn empty_material_is_rejected_after_the_gate() {
        let root = temp_root("empty");
        let store = TutorialStore::new(root.clone()).unwrap();
        let err = futures_lite_block_on(generate_review(
            &cfg_with_consent(),
            &store,
            &[],
            0,
            "01JPIPELINEEMPTY",
            None,
            ReviewCoverage::All,
        ))
        .unwrap_err();
        assert!(format!("{err:#}").contains("no reviewable decision points"));
        std::fs::remove_dir_all(root).ok();
    }

    /// Minimal block-on for tests: the pipeline is async only because the
    /// provider call is; everything these tests exercise runs
    /// synchronously up to (and excluding) the network.
    fn futures_lite_block_on<F: std::future::Future>(fut: F) -> F::Output {
        // A dedicated tiny runtime keeps this dependency-free and matches
        // the rest of the crate (tokio with rt).
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(fut)
    }
}
