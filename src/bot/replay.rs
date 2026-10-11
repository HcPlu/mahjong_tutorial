//! Offline replay of a recorded game through a subprocess bot (fork-only,
//! M4-lite): the "what would the bot have done" reference for the
//! tutorial replay page (D-0012's deferred bot-replay, pulled forward).
//!
//! The recorded stream already contains the human's actual moves, so the
//! bot's internal state stays consistent by simply feeding the stream in
//! order — no reach follow-up games (that trick exists for autoplay,
//! where the declaration must be committed up-front; see manager.rs).
//!
//! Feeding strategy: batch events up to and including our `Tsumo` at
//! each decision point, `react`, capture the answer (action + meta.show
//! probabilities), continue from the next event. One react call per own
//! decision (~70 per game), `none` never crosses the wire.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::bot::runner::{BotRunner, SubprocessBot};
use crate::bot::PythonRuntime;
use crate::event_bus::NotifyBus;
use crate::schema::MjaiEvent;

/// One bot-recommended tile with its policy probability, from
/// `meta.show` (same shape the HUD renders).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReplayPick {
    /// Mjai tile string (e.g. "5s").
    pub tile: String,
    /// Row label from the show card ("Discard", "Riichi", ...).
    pub label: String,
    /// Bot policy probability, 0..=1.
    pub prob: f64,
}

/// The bot's answer at one of our recorded decision points.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReplayDecision {
    /// Index of our `Tsumo` event in the fed stream — correlates with
    /// the tutorial replay outline.
    pub event_index: u32,
    /// The bot's action (Dahai / Reach / ...; `None` when the bot had
    /// nothing to say at this window).
    pub action: MjaiEvent,
    pub picks: Vec<ReplayPick>,
}

/// Walk a recorded stream, capturing the bot's answers at the given
/// decision points (`event_index` = our draw events). Generic over the
/// runner so tests can drive it without a subprocess.
pub async fn walk_decisions(
    runner: &mut dyn BotRunner,
    events: &[MjaiEvent],
    decision_indices: &[u32],
) -> Result<Vec<ReplayDecision>> {
    let mut out = Vec::with_capacity(decision_indices.len());
    let mut cursor = 0usize;
    for &di in decision_indices {
        let end = di as usize + 1;
        if end > events.len() {
            break;
        }
        let batch = &events[cursor..end];
        let resp = runner
            .react(batch)
            .await
            .with_context(|| format!("bot react failed at event {end}"))?;
        cursor = end;
        out.push(ReplayDecision {
            event_index: di,
            action: resp.action,
            picks: extract_picks(&resp.meta),
        });
    }
    Ok(out)
}

/// Spawn the configured subprocess bot and replay the whole game in one
/// go. The child is dropped (killed) on return — one-shot by design.
pub async fn replay_recorded_game(
    runtime: &PythonRuntime,
    bot_dir: &Path,
    actor_id: u8,
    notify_tx: NotifyBus,
    events: &[MjaiEvent],
    decision_indices: &[u32],
) -> Result<Vec<ReplayDecision>> {
    let mut runner = SubprocessBot::spawn(runtime, bot_dir, actor_id, notify_tx)
        .await
        .context("spawn replay bot")?;
    walk_decisions(&mut runner, events, decision_indices).await
}

/// `meta.show.items` → single-tile picks, in the bot's ranking order.
/// Rows naming more than one tile (or none — Pass/Riichi rows without a
/// tile) carry no discard probability for the chart and are skipped.
pub fn extract_picks(meta: &Option<Value>) -> Vec<ReplayPick> {
    let Some(items) = meta
        .as_ref()
        .and_then(|m| m.get("show"))
        .and_then(|s| s.get("items"))
        .and_then(|i| i.as_array())
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let pais = item.get("pais")?.as_array()?;
            if pais.len() != 1 {
                return None;
            }
            Some(ReplayPick {
                tile: pais[0].as_str()?.to_string(),
                label: item
                    .get("label")
                    .and_then(|l| l.as_str())
                    .unwrap_or("")
                    .to_string(),
                prob: item.get("prob").and_then(|p| p.as_f64()).unwrap_or(0.0),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::BotResponse;
    use crate::schema::MjaiEvent;
    use async_trait::async_trait;
    use serde_json::json;

    /// Records every batch it was fed and replays scripted responses.
    struct FakeRunner {
        batches: Vec<Vec<MjaiEvent>>,
        answers: Vec<BotResponse>,
    }

    #[async_trait]
    impl BotRunner for FakeRunner {
        async fn react(&mut self, events: &[MjaiEvent]) -> Result<BotResponse> {
            self.batches.push(events.to_vec());
            Ok(self
                .answers
                .get(self.batches.len() - 1)
                .cloned()
                .unwrap_or(BotResponse {
                    action: MjaiEvent::None,
                    meta: None,
                }))
        }
        async fn reset(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn ev(i: usize) -> MjaiEvent {
        // Distinct-ish events so batch boundaries are checkable.
        if i.is_multiple_of(2) {
            MjaiEvent::Tsumo {
                actor: (i % 4) as u8,
                pai: format!("{i}m"),
            }
        } else {
            MjaiEvent::Dahai {
                actor: (i % 4) as u8,
                pai: format!("{i}p"),
                tsumogiri: true,
            }
        }
    }

    #[tokio::test]
    async fn batches_end_exactly_at_decision_indices() {
        let events: Vec<MjaiEvent> = (0..10).map(ev).collect();
        let mut runner = FakeRunner {
            batches: vec![],
            answers: vec![],
        };
        let out = walk_decisions(&mut runner, &events, &[2, 5]).await.unwrap();
        // First batch: 0..=2, second: 3..=5 — nothing after the last
        // decision is fed (one react per decision point).
        assert_eq!(runner.batches.len(), 2);
        assert_eq!(runner.batches[0].len(), 3);
        assert_eq!(runner.batches[1].len(), 3);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].event_index, 2);
    }

    #[tokio::test]
    async fn out_of_range_indices_stop_cleanly() {
        let events: Vec<MjaiEvent> = (0..4).map(ev).collect();
        let mut runner = FakeRunner {
            batches: vec![],
            answers: vec![],
        };
        let out = walk_decisions(&mut runner, &events, &[1, 99])
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn extracts_single_tile_picks_in_rank_order() {
        let meta = Some(json!({
            "show": { "items": [
                { "label": "Discard", "pais": ["5s"], "prob": 0.62 },
                { "label": "Discard", "pais": ["1z"], "prob": 0.2 },
                { "label": "Riichi", "pais": [], "prob": 0.1 },
                { "label": "Chi (high)", "pais": ["1m", "2m"], "prob": 0.08 }
            ]}
        }));
        let picks = extract_picks(&meta);
        assert_eq!(picks.len(), 2);
        assert_eq!(picks[0].tile, "5s");
        assert_eq!(picks[0].label, "Discard");
        assert!((picks[0].prob - 0.62).abs() < 1e-9);
        // Missing show → empty; missing prob defaults to 0.
        assert!(extract_picks(&None).is_empty());
        let no_prob =
            Some(json!({ "show": { "items": [ { "label": "Discard", "pais": ["3p"] } ] } }));
        assert_eq!(extract_picks(&no_prob)[0].prob, 0.0);
    }
}
