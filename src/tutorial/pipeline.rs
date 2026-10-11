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
use crate::tutorial::download::{parse_review_doc_trust, parse_turn_doc_trust};
use crate::tutorial::material::{
    LiveTurnDoc, ReviewCoverage, ReviewDoc, ReviewMaterial, TurnMaterial,
};
use crate::tutorial::provider::{ChatRequest, OpenAiCompatProvider, TutorialProvider};
use crate::tutorial::store::TutorialStore;
use crate::tutorial::upload;

/// Turns per chat request. Measured against the real GLM endpoint on a
/// full-length game: one 15-turn chunk makes reasoning-style models
/// (thinking + JSON from the same budget) time out or starve the content
/// to empty on the free tier. Six turns keeps every request inside a
/// minute and makes transient 429s cheap to retry.
const TURNS_PER_REQUEST: usize = 6;
const TEMPERATURE: f32 = 0.2;
/// Headroom for reasoning models: thinking tokens come out of the same
/// budget before the JSON body — GLM-5.x reasoning needs a lot of it
/// (measured against the real endpoint; unused budget is not billed).
const MAX_TOKENS_PER_REQUEST: u32 = 16000;
/// Single-turn live deep-dive knobs (same rationale as above, sized for
/// one turn's JSON — see D-0008 / D-0014).
const LIVE_TEMPERATURE: f32 = 0.2;
const LIVE_MAX_TOKENS: u32 = 6000;

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

    // Bot reference (offline replay) rides along when cached: the prompt
    // then narrates 你 vs bot per turn. Pure store read — generating a
    // missing reference is the IPC layer's decision (it owns the runtime).
    let botref = match store.load_botref(game_id) {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!("botref load failed for {game_id}: {e:#} — reviewing without bot data");
            None
        }
    };
    let material = build_review_material(
        events,
        seat,
        game_id,
        player_name,
        coverage,
        botref.as_ref(),
    )
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
                // From-config hint (e.g. GLM-5.x thinking control); unset
                // → not sent.
                reasoning_effort: cfg.llm.reasoning_effort.clone(),
            })
            .await
            .context("LLM request failed — degrading to a numbers-only review")?;
        // Trust passthrough (D-0013, current stage): keep the reply
        // verbatim — no cross-checking, no turn dropping. The strict
        // validators stay parked in download.rs for when the stage ends.
        let doc = parse_review_doc_trust(&resp.content, &sub)
            .context("LLM reply was not keepable JSON — degrading to a numbers-only review")?;
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

/// Generate one live-turn deep-dive: provider → trust parse → stamp →
/// save (D-0008 manual button and D-0014 auto worker share this).
///
/// The caller owns the cache check and the consent gate — the IPC
/// command checks the cache *before* consent (cached turns are served
/// without consent), the auto worker re-reads config per job. The
/// reasoning-effort hint is left unset in the request so the provider
/// stamps its own `LlmConfig` default — same value, one source of truth.
pub async fn generate_live_turn(
    store: &TutorialStore,
    provider: &dyn TutorialProvider,
    material: &TurnMaterial,
    key: &str,
) -> Result<LiveTurnDoc> {
    let resp = provider
        .complete(ChatRequest {
            messages: upload::build_turn_messages(material),
            temperature: Some(LIVE_TEMPERATURE),
            // Headroom for reasoning models: thinking + the JSON body
            // share one budget (see MAX_TOKENS_PER_REQUEST above).
            max_tokens: Some(LIVE_MAX_TOKENS),
            reasoning_effort: None,
        })
        .await
        .context("LLM request failed")?;
    // Trust passthrough (D-0013, current stage): keep the reply
    // verbatim; this only extracts the JSON and stamps our junme.
    let doc =
        parse_turn_doc_trust(&resp.content, material).context("LLM reply was not keepable JSON")?;
    let live_doc = LiveTurnDoc {
        key: key.to_string(),
        doc,
        model: resp.model,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    store.save_turn_doc(&live_doc)?;
    Ok(live_doc)
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
