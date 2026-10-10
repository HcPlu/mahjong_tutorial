//! Live coach deep-dive (M2, D-0008): the backend half of the in-game
//! "explain this decision" button.
//!
//! A `drive_loop` task (spawned in `lib.rs` assembly) subscribes to the
//! `PostTrackerBus` (session/junme bookkeeping) and the `BotResponseBus`
//! (decision-point signal). When the bot answers, the latest decision is
//! re-derived **from the tracker snapshot** via the same pure chain the
//! analysis runner and the review assembler use — numbers come from the
//! local engine at the source (D-0005), never from the frontend, and
//! never from pairing two racing bus subscribers (the original design
//! paired AnalysisBus with BotResponseBus; two parallel subscribers have
//! no ordering guarantee, so the analysis is recomputed instead — the
//! discard-window search is ~1 ms).
//!
//! `actual` holds the **bot's** first-choice action here (the live turn
//! is explained before the student has acted); the review assembler
//! fills the student's actual discard instead.
//!
//! Boundary note: `crate::bot::BotResponse` is the public wire type of
//! the `BotResponseBus` (it also travels to the frontend via IPC). It is
//! the only bot symbol this module names — no bot internals are called.

use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::{broadcast, Mutex as AsyncMutex};
use tracing::debug;

use crate::analysis::snapshot_adapter::to_player_info;
use crate::analysis::{analyze, AnalysisResult, DiscardCandidate};
use crate::bot::BotResponse;
use crate::event_bus::TrackedEvent;
use crate::game_state::snapshot::GameStateSnapshot;
use crate::game_state::tracker::GameTracker;
use crate::schema::MjaiEvent;
use crate::tutorial::assemble::{engine_numbers, is_discard_window, turn_context};
use crate::tutorial::material::{CandidateNumbers, TurnMaterial};

/// Cap on candidates offered per live turn.
const MAX_LIVE_CANDIDATES: usize = 5;

/// The one decision the coach currently holds (bounded: a single slot,
/// every new decision overwrites the previous one).
#[derive(Debug, Clone, PartialEq)]
pub struct LiveTurn {
    /// Cache-key namespace for this game (`YYYYMMDD-HHMMSS-seat`); the
    /// per-turn store key is `"{session_key}-t{junme}"`.
    pub session_key: String,
    pub material: TurnMaterial,
}

/// Shared handle the IPC command reads; the drive loop writes.
#[derive(Clone, Default)]
pub struct LiveCoachHandle {
    inner: Arc<Mutex<Option<LiveTurn>>>,
}

impl LiveCoachHandle {
    pub fn current(&self) -> Option<LiveTurn> {
        self.inner.lock().expect("live coach lock poisoned").clone()
    }

    fn set(&self, turn: LiveTurn) {
        *self.inner.lock().expect("live coach lock poisoned") = Some(turn);
    }
}

/// Drive the live coach over the two buses until they close. Spawned in
/// `lib.rs` inside the Tauri runtime.
pub async fn drive_loop(
    mut post_rx: broadcast::Receiver<TrackedEvent>,
    mut bot_rx: broadcast::Receiver<BotResponse>,
    tracker: Arc<AsyncMutex<GameTracker>>,
    coach: LiveCoachHandle,
) {
    debug!("live coach subscribed to post-tracker + bot-response buses");
    let mut session_key = String::new();
    let mut seat: Option<u8> = None;
    let mut our_draws: u32 = 0;

    loop {
        tokio::select! {
            ev = post_rx.recv() => {
                match ev {
                    Ok(TrackedEvent { event, .. }) => match &event {
                        MjaiEvent::StartGame { id, .. } => {
                            seat = *id;
                            our_draws = 0;
                            session_key = new_session_key(*id);
                        }
                        MjaiEvent::Tsumo { actor, .. } if Some(*actor) == seat => {
                            our_draws += 1;
                        }
                        _ => {}
                    },
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        debug!("live coach post-tracker rx lagged by {n}");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            resp = bot_rx.recv() => {
                match resp {
                    Ok(resp) => {
                        let Some(seat) = seat else { continue };
                        let snap = tracker.lock().await.snapshot();
                        let Some(snap) = snap else { continue };
                        if let Some(material) =
                            build_live_turn(&snap, &resp, our_draws)
                        {
                            let _ = seat; // seat already bound above
                            coach.set(LiveTurn {
                                session_key: session_key.clone(),
                                material,
                            });
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        debug!("live coach bot-response rx lagged by {n}");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

/// Session key from the local clock + seat — path-safe by construction
/// (validate_game_id allows `[A-Za-z0-9_-]`). Live games have no ULID
/// until the history recorder finalises them (D-0008 §4.3).
fn new_session_key(seat: Option<u8>) -> String {
    let seat = seat.map(|s| s.to_string()).unwrap_or_else(|| "x".into());
    format!("{}-{seat}", chrono::Local::now().format("%Y%m%d-%H%M%S"))
}

/// Assemble the live `TurnMaterial` for the decision the bot just
/// answered. Pure (snapshot in, material out) so tests need no buses.
///
/// `None` when this isn't the observed seat's discard window (call
/// windows are v1-out-of-scope), the engine has no candidates, or the
/// response carries no discard action to anchor `actual` on.
pub fn build_live_turn(
    snap: &GameStateSnapshot,
    resp: &BotResponse,
    junme: u32,
) -> Option<TurnMaterial> {
    let seat = snap.our_seat?;
    if !is_discard_window(snap, seat) {
        return None;
    }
    let me = snap.players.get(seat as usize)?;
    let info = to_player_info(snap, seat).ok()?;
    let result = analyze(&info);
    let hand14 = result.hand14.as_ref()?;
    if hand14.maintain.is_empty() {
        return None;
    }

    // `actual` = the bot's first-choice discard: the action event when it
    // is a Dahai, else the top meta.show row's single tile (riichi turns
    // answer `reach`, with the predicted tile on the show card).
    let actual = match &resp.action {
        MjaiEvent::Dahai { pai, .. } => format!("dahai:{pai}"),
        _ => top_show_tile(&resp.meta).map(|t| format!("dahai:{t}"))?,
    };

    // Candidates: the bot's own ranked rows (meta.show) where the engine
    // has numbers for the tile; fall back to the engine's top rows when
    // the bot published no show card.
    let mut candidates: Vec<CandidateNumbers> = Vec::new();
    for (tile, prob) in show_tiles(&resp.meta) {
        if candidates.len() >= MAX_LIVE_CANDIDATES {
            break;
        }
        if let Some(c) = hand14.maintain.iter().find(|c| c.discard == tile) {
            candidates.push(CandidateNumbers {
                label: format!("dahai:{tile}"),
                prob,
                numbers: engine_numbers(c, &result),
            });
        }
    }
    if candidates.is_empty() {
        candidates = engine_top_candidates(hand14, &result);
    }

    let tehai = me.tehai.clone();
    let draw = me
        .drawn_tile
        .clone()
        .or_else(|| tehai.last().cloned())
        .unwrap_or_default();

    Some(TurnMaterial {
        junme,
        tehai,
        draw,
        actual,
        candidates,
        context: Some(turn_context(snap, me.riichi_declared)),
    })
}

/// Engine fallback: top maintain rows ranked by their own
/// mixed_waits_score share (same semantics as the review assembler).
fn engine_top_candidates(
    hand14: &crate::analysis::Hand14Result,
    result: &AnalysisResult,
) -> Vec<CandidateNumbers> {
    let picked: Vec<&DiscardCandidate> = hand14.maintain.iter().take(3).collect();
    let total: f64 = picked
        .iter()
        .map(|c| c.result.mixed_waits_score.max(0.0))
        .sum();
    picked
        .iter()
        .map(|c| CandidateNumbers {
            label: format!("dahai:{}", c.discard),
            prob: if total > 0.0 {
                c.result.mixed_waits_score.max(0.0) / total
            } else {
                0.0
            },
            numbers: engine_numbers(c, result),
        })
        .collect()
}

/// `(tile, prob)` pairs from `meta.show.items`, in the bot's ranking
/// order. Skips rows that don't name exactly one tile (Riichi/Pass/Chi
/// rows carry no discard numbers v1 can quote).
fn show_tiles(meta: &Option<Value>) -> Vec<(String, f64)> {
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
            let tile = pais[0].as_str()?.to_string();
            let prob = item.get("prob").and_then(|p| p.as_f64()).unwrap_or(0.0);
            Some((tile, prob))
        })
        .collect()
}

/// The top show row's single tile, when it has one.
fn top_show_tile(meta: &Option<Value>) -> Option<String> {
    show_tiles(meta).into_iter().next().map(|(t, _)| t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::snapshot::Phase;
    use serde_json::json;

    fn snapshot_our_hand(tehai: Vec<&str>, drawn: Option<&str>) -> GameStateSnapshot {
        let mk = |seat: u8, t: Vec<String>| crate::game_state::snapshot::PlayerSnapshot {
            seat,
            tehai: t,
            melds: vec![],
            river: vec![],
            score: 25_000,
            riichi_declared: false,
            riichi_stage: false,
            double_riichi: false,
            riichi_declaration_index: None,
            kita_tiles: vec![],
            drawn_tile: drawn.filter(|_| seat == 0).map(String::from),
        };
        GameStateSnapshot {
            bakaze: "E".into(),
            kyoku: 1,
            honba: 0,
            kyotaku: 0,
            oya: 0,
            current_player: 0,
            turn_count: 1,
            phase: Phase::WaitAct,
            is_done: false,
            num_players: 4,
            players: vec![
                mk(0, tehai.into_iter().map(String::from).collect()),
                mk(1, (0..13).map(|_| "?".into()).collect()),
                mk(2, (0..13).map(|_| "?".into()).collect()),
                mk(3, (0..13).map(|_| "?".into()).collect()),
            ],
            dora_markers: vec!["2m".into()],
            our_seat: Some(0),
        }
    }

    fn bot_dahai(pai: &str, meta: Option<Value>) -> BotResponse {
        BotResponse {
            action: MjaiEvent::Dahai {
                actor: 0,
                pai: pai.into(),
                tsumogiri: false,
            },
            meta,
        }
    }

    fn hand() -> Vec<&'static str> {
        // 14 tiles at the decision point; many maintain discards exist.
        vec![
            "1m", "2m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "S", "E",
        ]
    }

    #[test]
    fn builds_material_from_bot_show() {
        let meta = Some(json!({
            "show": { "title": "t", "items": [
                { "label": "Discard", "pais": ["E"], "prob": 0.7 },
                { "label": "Discard", "pais": ["S"], "prob": 0.2 },
                { "label": "Riichi", "pais": [], "prob": 0.1 }
            ]}
        }));
        let snap = snapshot_our_hand(hand(), Some("E"));
        let resp = bot_dahai("E", meta);
        let mat = build_live_turn(&snap, &resp, 5).expect("decision window");
        assert_eq!(mat.junme, 5);
        assert_eq!(mat.actual, "dahai:E");
        assert_eq!(mat.tehai.len(), 14);
        assert_eq!(mat.draw, "E");
        // The riichi row (no tile) is skipped; probs come from the bot.
        let labels: Vec<&str> = mat.candidates.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["dahai:E", "dahai:S"]);
        assert!((mat.candidates[0].prob - 0.7).abs() < 1e-9);
        assert!(mat.candidates[0].numbers.win_rate.is_some());
        assert!(mat.context.is_some());
    }

    #[test]
    fn falls_back_to_engine_top_when_no_show() {
        let snap = snapshot_our_hand(hand(), Some("E"));
        let resp = bot_dahai("E", None);
        let mat = build_live_turn(&snap, &resp, 3).expect("decision window");
        assert!(!mat.candidates.is_empty());
        assert!(mat.candidates.iter().all(|c| c.label.starts_with("dahai:")));
        let prob_sum: f64 = mat.candidates.iter().map(|c| c.prob).sum();
        // Shares normalize to 1 — unless every picked candidate scores 0
        // (possible for degenerate hands), in which case all are 0.
        assert!(
            (prob_sum - 1.0).abs() < 1e-9 || prob_sum == 0.0,
            "probs must normalize, got {prob_sum}: {:?}",
            mat.candidates.iter().map(|c| c.prob).collect::<Vec<_>>()
        );
    }

    #[test]
    fn reach_answer_anchors_actual_on_show_tile() {
        let meta = Some(json!({
            "show": { "items": [ { "label": "Riichi", "pais": ["E"], "prob": 0.9 } ]}
        }));
        let snap = snapshot_our_hand(hand(), Some("E"));
        let resp = BotResponse {
            action: MjaiEvent::Reach {
                actor: 0,
                pai: None,
            },
            meta,
        };
        let mat = build_live_turn(&snap, &resp, 9).expect("decision window");
        assert_eq!(mat.actual, "dahai:E");
    }

    #[test]
    fn non_discard_window_is_skipped() {
        // 13 tiles: not a discard window, regardless of the response.
        let mut snap = snapshot_our_hand(hand(), None);
        snap.players[0].tehai.pop();
        snap.players[0].drawn_tile = None;
        let resp = bot_dahai("E", None);
        assert!(build_live_turn(&snap, &resp, 1).is_none());
    }

    #[test]
    fn session_key_is_path_safe() {
        for seat in [None, Some(0u8), Some(3u8)] {
            let key = new_session_key(seat);
            assert!(
                key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "key not path-safe: {key}"
            );
        }
    }
}
