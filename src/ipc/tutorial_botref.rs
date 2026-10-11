//! Orchestration for the tutorial's bot reference ("what would the bot
//! have done at each of my decisions"), fork-only (D-0012 M4-lite).
//!
//! Lives in the ipc layer because it bridges three modules whose
//! dependency directions must not bend: `bot` (runner + replay),
//! `history` (recorded stream), `tutorial` (persistence + types).
//! `src/tutorial/` itself stays free of bot imports (AGENTS §6).

use anyhow::{bail, Context, Result};

use crate::bot::replay::ReplaySource;
use crate::bot::PythonRuntime;
use crate::config::AppConfig;
use crate::event_bus::NotifyBus;
use crate::history::HistoryStore;
use crate::schema::MjaiEvent;
use crate::tutorial::material::{BotRefDoc, BotRefPick, BotRefSource, BotRefTurn};
use crate::tutorial::store::TutorialStore;
use crate::util::resolve_dir;
use std::path::Path;

/// Generate (or return the cached) bot reference for a recorded game:
/// replays the whole stream through the configured subprocess bot and
/// captures its answer + candidate probabilities at every own-discard
/// decision. No LLM involved — this is pure local bot inference.
pub async fn generate_bot_reference(
    cfg: &AppConfig,
    runtime: Option<PythonRuntime>,
    store: &TutorialStore,
    history: &HistoryStore,
    game_id: &str,
    notify: NotifyBus,
) -> Result<BotRefDoc> {
    // Cache before replay — the replay is the expensive part (one bot
    // inference per decision point).
    if let Some(doc) = store.load_botref(game_id)? {
        return Ok(doc);
    }

    let record = history
        .get(game_id)?
        .with_context(|| format!("unknown game id {game_id:?}"))?;
    let events = history
        .get_events(game_id)?
        .with_context(|| format!("no event stream recorded for game {game_id:?}"))?;
    let seat = record.our_seat.unwrap_or(0);

    let bot_name = if record.num_players == 3 {
        &cfg.bot.active_3p
    } else {
        &cfg.bot.active_4p
    };
    if bot_name.is_empty() {
        bail!(
            "no active bot configured for {}-player games (Settings → Bots)",
            record.num_players
        );
    }
    let bot_dir = resolve_dir(Path::new(&cfg.bot.dir)).join(bot_name);
    if !bot_dir.join("bot.py").exists() {
        bail!(
            "bot {bot_name:?} has no bot.py — the reference column needs a \
             subprocess bot (e.g. mortal); built-in native bots are not \
             replayable yet"
        );
    }
    let runtime = runtime.context("no Python runtime available for the bot replay")?;

    let scan = crate::tutorial::scan_decisions(&events, seat).context("scan recorded decisions")?;
    let indices: Vec<u32> = scan.decisions.iter().map(|d| d.event_index).collect();
    let started = std::time::Instant::now();
    let decisions = crate::bot::replay::replay_recorded_game(
        &runtime, &bot_dir, seat, notify, &events, &indices,
    )
    .await
    .context("bot replay failed")?;

    let turns: Vec<BotRefTurn> = scan
        .decisions
        .iter()
        .filter_map(|d| {
            let r = decisions.iter().find(|r| r.event_index == d.event_index)?;
            Some(BotRefTurn {
                junme: d.junme,
                action: action_label(&r.action),
                picks: r
                    .picks
                    .iter()
                    .map(|p| BotRefPick {
                        tile: p.tile.clone(),
                        label: p.label.clone(),
                        prob: p.prob,
                        chosen: p.chosen,
                    })
                    .collect(),
                source: Some(map_source(r.source)),
            })
        })
        .collect();

    let doc = BotRefDoc {
        game_id: game_id.to_string(),
        bot: bot_name.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        turns,
    };
    store.save_botref(&doc)?;
    tracing::info!(
        game = game_id,
        turns = doc.turns.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "bot reference replayed"
    );
    Ok(doc)
}

/// Coarse action label in the same vocabulary as the material's
/// `CandidateNumbers::label` (`dahai:5s`, `reach`, ...).
fn action_label(ev: &MjaiEvent) -> String {
    match ev {
        MjaiEvent::Dahai { pai, .. } => format!("dahai:{pai}"),
        MjaiEvent::Reach { .. } => "reach".into(),
        MjaiEvent::None => "none".into(),
        other => serde_json::to_value(other)
            .ok()
            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(String::from))
            .unwrap_or_else(|| "other".into()),
    }
}

/// `bot::replay`'s classification → the tutorial-side enum. The two
/// live on opposite sides of the bot/tutorial boundary and must stay
/// variant-for-variant in sync (this match has no `_` arm on purpose).
fn map_source(source: ReplaySource) -> BotRefSource {
    match source {
        ReplaySource::Model => BotRefSource::Model,
        ReplaySource::RuleForced => BotRefSource::RuleForced,
        ReplaySource::RuleOverridden => BotRefSource::RuleOverridden,
        ReplaySource::NoAnswer => BotRefSource::NoAnswer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_labels_use_material_vocabulary() {
        assert_eq!(
            action_label(&MjaiEvent::Dahai {
                actor: 0,
                pai: "5s".into(),
                tsumogiri: false
            }),
            "dahai:5s"
        );
        assert_eq!(
            action_label(&MjaiEvent::Reach {
                actor: 0,
                pai: None
            }),
            "reach"
        );
        assert_eq!(action_label(&MjaiEvent::None), "none");
        assert_eq!(action_label(&MjaiEvent::EndKyoku), "end_kyoku");
    }
}
