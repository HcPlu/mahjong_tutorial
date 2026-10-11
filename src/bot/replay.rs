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

/// How the bot produced one answer. Distinguishes real inference from
/// the rule-based fast paths so the tutorial can label it (mortal:
/// quick_eval / agari guard — see docs/design/mortal-internals.md §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplaySource {
    /// Neural-net inference picked the action (its top-ranked
    /// candidate under greedy argmax).
    Model,
    /// Rules forced the only legal action — no probability
    /// distribution exists (quick_eval: single discard candidate, the
    /// engine skips the network entirely and the reaction carries no
    /// `meta`).
    RuleForced,
    /// The net ranked one action first but a rule guard overrode it
    /// (agari guard: forced minogashi in all-last) — the chosen row is
    /// not the net's top.
    RuleOverridden,
    /// No usable answer at this window.
    NoAnswer,
}

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
    /// True on the row the bot actually played. Only set when the
    /// chosen action appears among the rows (discard turns); a forced
    /// discard (`RuleForced`) carries no rows at all.
    pub chosen: bool,
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
    /// Whether the action came from the model or a rule fast path.
    pub source: ReplaySource,
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
        let (source, picks) = classify_reaction(&resp.action, &resp.meta);
        out.push(ReplayDecision {
            event_index: di,
            action: resp.action,
            picks,
            source,
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

/// Classify one bot reaction: which candidate row the bot actually
/// played (`chosen`), and whether that answer came from the model or a
/// rule fast path.
///
/// Detection, against the mortal wire format (`q_values` + `mask_bits`
/// translated by the bot wrapper into `meta.show`; `is_greedy` rides
/// along untouched):
///
/// * `none` reply → `NoAnswer` (nothing owed at this window).
/// * reply without `meta` → `RuleForced`: mortal's quick_eval answers
///   carry no meta, and a real inference always does.
/// * chosen row present, greedy, and not the top row →
///   `RuleOverridden` (agari guard forces the second-best action).
/// * otherwise → `Model`.
///
/// Sampling (`is_greedy: false`) also allows chosen ≠ top and stays
/// `Model` — only a greedy non-top choice proves an override.
pub fn classify_reaction(
    action: &MjaiEvent,
    meta: &Option<Value>,
) -> (ReplaySource, Vec<ReplayPick>) {
    if matches!(action, MjaiEvent::None) {
        return (ReplaySource::NoAnswer, Vec::new());
    }
    let Some(items) = meta
        .as_ref()
        .and_then(|m| m.get("show"))
        .and_then(|s| s.get("items"))
        .and_then(|i| i.as_array())
    else {
        // No meta at all = quick_eval's forced discard (the fast path
        // only ever discards); anything else without a readable show is
        // inference whose distribution we can't see.
        let source = if meta.is_none() && matches!(action, MjaiEvent::Dahai { .. }) {
            ReplaySource::RuleForced
        } else {
            ReplaySource::Model
        };
        return (source, Vec::new());
    };

    let chosen_item = find_chosen_item(action, items);
    let is_greedy = meta
        .as_ref()
        .and_then(|m| m.get("is_greedy"))
        .and_then(|g| g.as_bool())
        .unwrap_or(true);
    let source = match chosen_item {
        Some(i) if is_greedy && i > 0 => ReplaySource::RuleOverridden,
        _ => ReplaySource::Model,
    };
    let picks = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
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
                chosen: chosen_item == Some(i),
            })
        })
        .collect();
    (source, picks)
}

/// Index of the show-card row the bot actually played. Discard actions
/// match by tile (red fives included: the engine's `pai` and the card's
/// `pais` both use the mjai aka notation, e.g. "5mr"); the few
/// non-discard rows match by the wrapper's fixed labels.
fn find_chosen_item(action: &MjaiEvent, items: &[Value]) -> Option<usize> {
    let (want_tile, want_label): (Option<&str>, Option<&str>) = match action {
        MjaiEvent::Dahai { pai, .. } => (Some(pai.as_str()), None),
        MjaiEvent::Reach { .. } => (None, Some("Riichi")),
        MjaiEvent::Hora { .. } => (None, Some("Agari")),
        MjaiEvent::Ryukyoku { .. } => (None, Some("Kyuushu")),
        _ => return None,
    };
    items.iter().position(|item| {
        if let Some(want) = want_label {
            item.get("label").and_then(|l| l.as_str()) == Some(want)
        } else {
            item.get("pais")
                .and_then(|p| p.as_array())
                .is_some_and(|p| p.len() == 1 && p[0].as_str() == want_tile)
        }
    })
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

    fn dahai(pai: &str) -> MjaiEvent {
        MjaiEvent::Dahai {
            actor: 0,
            pai: pai.into(),
            tsumogiri: false,
        }
    }

    #[test]
    fn classifies_model_turn_and_marks_the_chosen_row() {
        // Greedy argmax: the engine's dahai is the show card's rank-1
        // row. Non-tile rows (Riichi/Chi) stay out of the picks but
        // still count for rank comparison.
        let meta = Some(json!({
            "is_greedy": true,
            "show": { "items": [
                { "label": "Discard 5s", "pais": ["5s"], "prob": 0.62 },
                { "label": "Discard 1z", "pais": ["1z"], "prob": 0.2 },
                { "label": "Riichi", "pais": [], "prob": 0.1 },
                { "label": "Chi (high)", "pais": ["1m", "2m"], "prob": 0.08 }
            ]}
        }));
        let (source, picks) = classify_reaction(&dahai("5s"), &meta);
        assert_eq!(source, ReplaySource::Model);
        assert_eq!(picks.len(), 2);
        assert_eq!(picks[0].tile, "5s");
        assert!(picks[0].chosen);
        assert!(!picks[1].chosen);
    }

    #[test]
    fn matches_red_five_by_aka_notation() {
        let meta = Some(json!({
            "is_greedy": true,
            "show": { "items": [
                { "label": "Discard 5mr", "pais": ["5mr"], "prob": 0.8 },
                { "label": "Discard 5m", "pais": ["5m"], "prob": 0.1 }
            ]}
        }));
        let (source, picks) = classify_reaction(&dahai("5mr"), &meta);
        assert_eq!(source, ReplaySource::Model);
        assert!(picks[0].chosen);
        assert!(!picks[1].chosen);
    }

    #[test]
    fn classifies_rule_forced_when_meta_is_absent() {
        // quick_eval answers carry no meta at all.
        let (source, picks) = classify_reaction(&dahai("3p"), &None);
        assert_eq!(source, ReplaySource::RuleForced);
        assert!(picks.is_empty());
        // But a `none` reply is never a rule decision.
        let (source, picks) = classify_reaction(&MjaiEvent::None, &None);
        assert_eq!(source, ReplaySource::NoAnswer);
        assert!(picks.is_empty());
    }

    #[test]
    fn classifies_agari_guard_override_when_greedy_choice_is_not_top() {
        // All-last minogashi: the net ranked Agari first (a label row,
        // not a tile row) but the guard forced the second-best discard.
        let meta = Some(json!({
            "is_greedy": true,
            "show": { "items": [
                { "label": "Agari", "pais": [], "prob": 0.7 },
                { "label": "Discard 5s", "pais": ["5s"], "prob": 0.25 }
            ]}
        }));
        let (source, picks) = classify_reaction(&dahai("5s"), &meta);
        assert_eq!(source, ReplaySource::RuleOverridden);
        assert_eq!(picks.len(), 1);
        assert!(picks[0].chosen);
        // Sampling produces the same shape and stays Model.
        let sampled = Some(json!({
            "is_greedy": false,
            "show": { "items": [
                { "label": "Agari", "pais": [], "prob": 0.7 },
                { "label": "Discard 5s", "pais": ["5s"], "prob": 0.25 }
            ]}
        }));
        let (source, _) = classify_reaction(&dahai("5s"), &sampled);
        assert_eq!(source, ReplaySource::Model);
    }

    #[test]
    fn classifies_non_discard_actions_by_label() {
        let meta = Some(json!({
            "is_greedy": true,
            "show": { "items": [ { "label": "Riichi", "pais": [], "prob": 0.9 } ]}
        }));
        let (source, picks) = classify_reaction(
            &MjaiEvent::Reach {
                actor: 0,
                pai: None,
            },
            &meta,
        );
        assert_eq!(source, ReplaySource::Model);
        // The Riichi row carries no tile — no picks, but no override
        // either (it *is* the top row).
        assert!(picks.is_empty());
    }

    #[test]
    fn missing_prob_defaults_to_zero() {
        let meta = Some(json!({ "show": { "items": [ { "label": "Discard", "pais": ["3p"] } ] } }));
        let (_, picks) = classify_reaction(&dahai("3p"), &meta);
        assert_eq!(picks[0].prob, 0.0);
    }
}
