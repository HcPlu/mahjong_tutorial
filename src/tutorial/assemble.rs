//! Review material assembler (M1, D-0012): turns a recorded mjai event
//! stream from `crate::history` into a [`ReviewMaterial`] the three-zone
//! pipeline can explain.
//!
//! Reads only (`game_state` for replay, `analysis` for numbers, `schema`
//! for events) — this module never touches the network, disk, or
//! `crate::ipc` / `crate::bot` internals.
//!
//! ## Decision points
//!
//! A decision point is our seat's *discard* window: the events from our
//! `Tsumo` up to (not including) our next `Dahai`. Meld / kan / riichi
//! call windows are decision points too, but [`TurnMaterial`] models
//! discards only — v1 teaches discard choices (see D-0012 for the
//! boundary). A draw that ends in `Hora` / `Ryukyoku` / `EndKyoku`
//! without our discard produced no decision and is skipped.
//!
//! `junme` counts *our* draws (the Mortal convention), not the snapshot's
//! global `turn_count`.
//!
//! ## Key turns
//!
//! `ReviewCoverage::Key` keeps the decision points selected by
//! [`is_key_turn`] — a deterministic, numbers-only ruleset (efficiency
//! misses, push/fold situations, first tenpai, attack/defence splits).
//! The same flags drive the replay outline badges in the frontend.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::analysis::snapshot_adapter::to_player_info;
use crate::analysis::{analyze, AnalysisResult, DiscardCandidate, Tile34};
use crate::game_state::tracker::GameTracker;
use crate::schema::MjaiEvent;
use crate::tutorial::material::{
    CandidateNumbers, EngineNumbers, ReviewCoverage, ReviewMaterial, TurnContext, TurnMaterial,
};

/// How many of `hand14.maintain`'s top-ranked candidates always make it
/// into the material.
const KEY_TOP_N: usize = 3;
/// Hard cap on candidates per turn (top-N + actual + defence pick).
const MAX_CANDIDATES: usize = 5;

/// One recognised decision point: the material turn plus the facts the
/// outline / key-turn selector need.
#[derive(Debug, Clone)]
pub struct DecisionPoint {
    /// Index of our draw (`Tsumo`) event in the source stream — the
    /// replay stepper jumps to this.
    pub event_index: u32,
    pub junme: u32,
    pub material: TurnMaterial,
    pub is_key: bool,
}

/// Everything `scan_decisions` learned about one stream.
pub struct ScanResult {
    pub num_players: u8,
    pub decisions: Vec<DecisionPoint>,
}

struct PendingDraw {
    event_index: u32,
    junme: u32,
    /// We declared riichi between the draw and the discard (`Reach`
    /// precedes the `Dahai` in mjai).
    declared_riichi: bool,
}

/// Replay `events` through a private [`GameTracker`] and collect every
/// discard decision of `seat` as a [`DecisionPoint`].
///
/// The tracker takes its observer seat from `StartGame.id`; history
/// streams recorded without a perspective would snapshot every hand as
/// `"?"`, so a missing id is patched to `seat` before the event is fed
/// (a per-event clone — streams are a few thousand small enums at most).
pub fn scan_decisions(events: &[MjaiEvent], seat: u8) -> Result<ScanResult> {
    let mut tracker = GameTracker::new();
    let mut num_players = 4u8;
    let mut decisions = Vec::new();
    let mut our_draws = 0u32;
    let mut pending: Option<PendingDraw> = None;
    // First tenpai per kyoku feeds key rule ③; StartKyoku resets it.
    let mut tenpai_seen = false;

    for (i, ev) in events.iter().enumerate() {
        let idx = i as u32;
        match ev {
            MjaiEvent::StartGame {
                id,
                num_players: np,
                ..
            } => {
                num_players = *np;
                our_draws = 0;
                tenpai_seen = false;
                pending = None;
                if id.is_none() {
                    let mut patched = ev.clone();
                    if let MjaiEvent::StartGame { id, .. } = &mut patched {
                        *id = Some(seat);
                    }
                    tracker.handle(&patched)?;
                } else {
                    tracker.handle(ev)?;
                }
            }
            MjaiEvent::StartKyoku { .. } => {
                tenpai_seen = false;
                pending = None;
                tracker.handle(ev)?;
            }
            MjaiEvent::Tsumo { actor, .. } if *actor == seat => {
                our_draws += 1;
                tracker.handle(ev)?;
                pending = Some(PendingDraw {
                    event_index: idx,
                    junme: our_draws,
                    declared_riichi: false,
                });
            }
            MjaiEvent::Reach { actor, .. } if *actor == seat => {
                if let Some(p) = pending.as_mut() {
                    p.declared_riichi = true;
                }
                tracker.handle(ev)?;
            }
            MjaiEvent::Dahai { actor, pai, .. } => {
                if *actor == seat {
                    if let Some(p) = pending.take() {
                        if let Some(dp) = build_decision(&tracker, seat, &p, pai, &mut tenpai_seen)?
                        {
                            decisions.push(dp);
                        }
                    }
                }
                tracker.handle(ev)?;
            }
            // Round endings invalidate any open draw window (tsumo agari,
            // abortive draws, trailing events after the last discard).
            MjaiEvent::Hora { .. } | MjaiEvent::Ryukyoku { .. } | MjaiEvent::EndKyoku => {
                pending = None;
                tracker.handle(ev)?;
            }
            _ => tracker.handle(ev)?,
        }
    }

    Ok(ScanResult {
        num_players,
        decisions,
    })
}

/// Assemble the review pipeline's input for one game and seat, keeping
/// the turns selected by `coverage`.
pub fn build_review_material(
    events: &[MjaiEvent],
    seat: u8,
    game_id: &str,
    player_name: Option<String>,
    coverage: ReviewCoverage,
) -> Result<ReviewMaterial> {
    let ScanResult {
        num_players,
        decisions,
    } = scan_decisions(events, seat)?;
    let turns = match coverage {
        ReviewCoverage::Key => decisions
            .iter()
            .filter(|d| d.is_key)
            .map(|d| d.material.clone())
            .collect(),
        ReviewCoverage::All => decisions.iter().map(|d| d.material.clone()).collect(),
    };
    Ok(ReviewMaterial {
        game_id: game_id.to_string(),
        seat,
        num_players,
        player_name,
        turns,
    })
}

/// Replay `events[0..=upto]` through a private tracker (with the same
/// `StartGame.id` injection as [`scan_decisions`]) and return the
/// snapshot at that point. `None` when `upto` predates the first
/// `StartGame`/`StartKyoku` (no game state yet).
///
/// Whole-stream replay per call is deliberate: a full game is a few
/// thousand events at µs each, so per-step statefulness buys nothing and
/// avoids cache-invalidation bugs.
pub fn replay_snapshot_at(
    events: &[MjaiEvent],
    seat: u8,
    upto: usize,
) -> Result<Option<crate::game_state::snapshot::GameStateSnapshot>> {
    if events.len() <= upto {
        anyhow::bail!(
            "event index {upto} out of range (stream has {})",
            events.len()
        );
    }
    let mut tracker = GameTracker::new();
    for ev in events.iter().take(upto + 1) {
        if let MjaiEvent::StartGame { id: None, .. } = ev {
            let mut patched = ev.clone();
            if let MjaiEvent::StartGame { id, .. } = &mut patched {
                *id = Some(seat);
            }
            tracker.handle(&patched)?;
        } else {
            tracker.handle(ev)?;
        }
    }
    Ok(tracker.snapshot())
}

/// Whether a snapshot is the observed seat's discard window (drew, must
/// discard): our turn, waiting on an action, holding a 3k+2-length hand
/// (14 tiles at zero melds; each meld shrinks the raw count by 3).
pub fn is_discard_window(snap: &crate::game_state::snapshot::GameStateSnapshot, seat: u8) -> bool {
    snap.current_player == seat
        && matches!(snap.phase, crate::game_state::snapshot::Phase::WaitAct)
        && snap
            .players
            .get(seat as usize)
            .is_some_and(|p| p.tehai.len() % 3 == 2)
}

/// Lightweight scan summary for the frontend replay stepper: where the
/// reviewed seat's decision points are and which are key turns.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplayOutline {
    pub seat: u8,
    pub num_players: u8,
    pub total_events: u32,
    pub decisions: Vec<OutlineDecision>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutlineDecision {
    /// Index of our draw (`Tsumo`) event in the stream.
    pub event_index: u32,
    pub junme: u32,
    pub is_key: bool,
    /// The student's actual discard, mjai tile (e.g. `"5s"`) — lets the
    /// replay page badge the real pick on the candidate chart without
    /// shipping the whole material.
    pub actual: String,
}

/// One replay step for the frontend: the snapshot at an event index,
/// its pre-encoded mahgen view, and — when the index lands on the
/// reviewed seat's discard window — the local analysis of that decision.
///
/// Serialize-only: `AnalysisResult` is not deserializable, and this type
/// only ever travels backend → frontend.
#[derive(Debug, Clone, Serialize)]
pub struct ReplayStep {
    pub snapshot: crate::game_state::snapshot::GameStateSnapshot,
    pub mahgen: crate::game_state::mahgen_view::MahgenView,
    pub analysis: Option<crate::analysis::result::AnalysisResult>,
}

impl ScanResult {
    /// Collapse a scan into the IPC outline payload.
    pub fn to_outline(&self, seat: u8, total_events: usize) -> ReplayOutline {
        ReplayOutline {
            seat,
            num_players: self.num_players,
            total_events: total_events as u32,
            decisions: self
                .decisions
                .iter()
                .map(|d| OutlineDecision {
                    event_index: d.event_index,
                    junme: d.junme,
                    is_key: d.is_key,
                    actual: d
                        .material
                        .actual
                        .strip_prefix("dahai:")
                        .unwrap_or(&d.material.actual)
                        .to_string(),
                })
                .collect(),
        }
    }
}

/// Build one decision point from the tracker state *between* our draw
/// and our discard (the snapshot still holds 14 tiles). Returns `Ok(None)`
/// for turns the engine has nothing to say about (no `maintain`
/// candidates) — those must not reach the material, because the download
/// zone cannot validate a candidate-less turn.
fn build_decision(
    tracker: &GameTracker,
    seat: u8,
    pending: &PendingDraw,
    actual: &str,
    tenpai_seen: &mut bool,
) -> Result<Option<DecisionPoint>> {
    let Some(snap) = tracker.snapshot() else {
        return Ok(None);
    };
    if snap.our_seat != Some(seat) {
        return Ok(None);
    }
    let Some(me) = snap.players.get(seat as usize) else {
        return Ok(None);
    };

    let info = to_player_info(&snap, seat)
        .with_context(|| format!("adapter rejected decision at junme {}", pending.junme))?;
    let result = analyze(&info);
    let Some(hand14) = result.hand14.as_ref() else {
        return Ok(None);
    };
    if hand14.maintain.is_empty() {
        return Ok(None);
    }

    // Candidate set: engine top-N, plus the student's actual discard,
    // plus the defence-best discard — each only when the engine has a
    // maintain line for it (otherwise there are no numbers to quote).
    let mut picked: Vec<&DiscardCandidate> = hand14.maintain.iter().take(KEY_TOP_N).collect();
    if let Some(c) = hand14.maintain.iter().find(|c| c.discard == actual) {
        push_unique(&mut picked, c);
    }
    if let Some(def) = result
        .best_defence_discard
        .as_deref()
        .and_then(|t| hand14.maintain.iter().find(|c| c.discard == t))
    {
        push_unique(&mut picked, def);
    }
    picked.truncate(MAX_CANDIDATES);

    // `prob` is the engine's own ranking pressure: each candidate's share
    // of the shown candidates' mixed_waits_score (D-0012 — an
    // engine-derived hint, not a bot action probability).
    let score_total: f64 = picked
        .iter()
        .map(|c| c.result.mixed_waits_score.max(0.0))
        .sum();
    let candidates: Vec<CandidateNumbers> = picked
        .iter()
        .map(|c| CandidateNumbers {
            label: format!("dahai:{}", c.discard),
            prob: if score_total > 0.0 {
                c.result.mixed_waits_score.max(0.0) / score_total
            } else {
                0.0
            },
            numbers: engine_numbers(c, &result),
        })
        .collect();

    // Key-turn facts.
    let actual_rank = hand14.maintain.iter().position(|c| c.discard == actual);
    let opp_riichi = snap
        .players
        .iter()
        .any(|p| p.seat != seat && p.riichi_declared);
    let best_shanten = hand14
        .maintain
        .first()
        .map(|c| c.result.shanten)
        .unwrap_or(hand14.shanten);
    let first_tenpai = best_shanten == 0 && !*tenpai_seen;
    if best_shanten == 0 {
        *tenpai_seen = true;
    }
    let candidate_tiles: Vec<&str> = picked.iter().map(|c| c.discard.as_str()).collect();
    let facts = KeyFacts {
        actual_rank,
        opp_riichi,
        best_shanten,
        first_tenpai,
        attack: result.best_attack_discard.clone(),
        defence: result.best_defence_discard.clone(),
        candidate_tiles: &candidate_tiles,
    };

    let context = turn_context(&snap, me.riichi_declared || pending.declared_riichi);
    let tehai = me.tehai.clone();
    let draw = me
        .drawn_tile
        .clone()
        .or_else(|| tehai.last().cloned())
        .unwrap_or_default();

    Ok(Some(DecisionPoint {
        event_index: pending.event_index,
        junme: pending.junme,
        material: TurnMaterial {
            junme: pending.junme,
            tehai,
            draw,
            actual: format!("dahai:{actual}"),
            candidates,
            context: Some(context),
        },
        is_key: is_key_turn(&facts),
    }))
}

/// Dedup-by-tile helper for the candidate pick list (both borrows tie to
/// the same `hand14` lifetime).
fn push_unique<'a>(list: &mut Vec<&'a DiscardCandidate>, c: &'a DiscardCandidate) {
    if !list.iter().any(|x| x.discard == c.discard) {
        list.push(c);
    }
}

/// Snapshot → situational frame. Shared by the review assembler and the
/// live coach. `our_riichi` includes a riichi declared this turn (the
/// `Reach` event lands between draw and discard).
pub(crate) fn turn_context(
    snap: &crate::game_state::snapshot::GameStateSnapshot,
    our_riichi: bool,
) -> TurnContext {
    TurnContext {
        bakaze: snap.bakaze.clone(),
        kyoku: snap.kyoku,
        honba: snap.honba,
        scores: snap.players.iter().map(|p| p.score).collect(),
        riichi_seats: snap
            .players
            .iter()
            .filter(|p| p.riichi_declared)
            .map(|p| p.seat)
            .collect(),
        dora_indicators: snap.dora_markers.clone(),
        our_riichi,
    }
}

/// Probabilities are quantized **at the source** to lengths models echo
/// reliably (win rate 2 decimals, deal-in 3): a real reply rounded
/// `0.017039` to `0.017` and failed exact-equality validation, and
/// 1e-2/1e-3 resolution is far beyond teaching needs. Quantizing keeps
/// the download zone's check strict — both sides carry the same rounded
/// value.
pub(crate) fn engine_numbers(cand: &DiscardCandidate, result: &AnalysisResult) -> EngineNumbers {
    let r = &cand.result;
    // mixed_risk / avg_agari_rate are percent (0..100) in the engine;
    // EngineNumbers is 0.0..=1.0.
    let deal_in_rate = Tile34::from_mjai(&cand.discard)
        .and_then(|t| result.mixed_risk.get(t.idx() as usize).copied())
        .map(|pct| quantize(pct / 100.0, 3));
    let avg_score = if r.riichi_point > 0.0 {
        Some(r.riichi_point.round() as i64)
    } else if r.dama_point > 0.0 {
        Some(r.dama_point.round() as i64)
    } else {
        None
    };
    EngineNumbers {
        shanten: i32::from(r.shanten),
        win_rate: Some(quantize(r.avg_agari_rate / 100.0, 2)),
        deal_in_rate,
        avg_score,
    }
}

/// Round to `places` decimal places — see [`engine_numbers`] for why the
/// checked numbers are quantized at the source.
pub(crate) fn quantize(v: f64, places: u32) -> f64 {
    let scale = 10f64.powi(places as i32);
    (v * scale).round() / scale
}

/// The inputs of [`is_key_turn`] — kept as a plain struct so the rule
/// set is table-testable without replaying a game.
pub(crate) struct KeyFacts<'a> {
    /// Rank of the student's actual discard in `maintain`; `None` when
    /// the discard wasn't a same-shanten option at all.
    pub actual_rank: Option<usize>,
    pub opp_riichi: bool,
    /// Best post-discard shanten reachable this turn.
    pub best_shanten: i8,
    /// First turn of this kyoku that can discard into tenpai.
    pub first_tenpai: bool,
    pub attack: Option<String>,
    pub defence: Option<String>,
    /// Tiles of the candidates offered for this turn.
    pub candidate_tiles: &'a [&'a str],
}

/// Deterministic key-turn selection (all inputs are engine numbers).
///
/// ① efficiency miss — actual not in the engine's top-3 (or not a
///    same-shanten discard at all);
/// ② push/fold — an opponent is in riichi while we are at most
///    1-shanten (attack vs defence is live);
/// ③ tenpai declaration — first discard-to-tenpai of the kyoku;
/// ④ attack/defence split — the engine's attack-best and defence-best
///    differ and both are among the offered candidates.
pub(crate) fn is_key_turn(f: &KeyFacts) -> bool {
    let missed_efficiency = f.actual_rank.is_none_or(|r| r >= KEY_TOP_N);
    let push_fold = f.opp_riichi && f.best_shanten <= 1;
    let attack_defence_split = match (&f.attack, &f.defence) {
        (Some(a), Some(d)) if a != d => {
            f.candidate_tiles.contains(&a.as_str()) && f.candidate_tiles.contains(&d.as_str())
        }
        _ => false,
    };
    missed_efficiency || push_fold || f.first_tenpai || attack_defence_split
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tutorial::material::ReviewCoverage;

    fn start_game(id: Option<u8>) -> MjaiEvent {
        MjaiEvent::StartGame {
            names: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            kyoku_first: None,
            aka_flag: None,
            id,
            num_players: 4,
            game_meta: None,
        }
    }

    /// Real 13-tile hand for seat 0, `?` for the seats we cannot see
    /// (the same convention the tracker's own tests use). Deliberately
    /// NOT a complete shape after the fixture draws: a complete hand has
    /// an empty `maintain` list (every discard walks back from agari) and
    /// the assembler correctly drops the turn.
    fn start_kyoku() -> MjaiEvent {
        let ours: Vec<String> = [
            "1m", "2m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "S",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let hidden: Vec<String> = (0..13).map(|_| "?".into()).collect();
        MjaiEvent::StartKyoku {
            bakaze: "E".into(),
            dora_marker: "2m".into(),
            kyoku: 1,
            honba: 0,
            kyotaku: 0,
            oya: 0,
            scores: vec![25_000, 25_000, 25_000, 25_000],
            tehais: vec![ours, hidden.clone(), hidden.clone(), hidden],
            num_players: 4,
        }
    }

    fn tsumo(actor: u8, pai: &str) -> MjaiEvent {
        MjaiEvent::Tsumo {
            actor,
            pai: pai.into(),
        }
    }

    fn dahai(actor: u8, pai: &str, tsumogiri: bool) -> MjaiEvent {
        MjaiEvent::Dahai {
            actor,
            pai: pai.into(),
            tsumogiri,
        }
    }

    /// Two full turns for seat 0 with opponents drawing/discarding in
    /// between, then the round ends.
    fn small_game() -> Vec<MjaiEvent> {
        vec![
            start_game(None),
            start_kyoku(),
            // Turn 1: we draw E and discard it (tsumogiri).
            tsumo(0, "E"),
            dahai(0, "E", true),
            tsumo(1, "9p"),
            dahai(1, "9p", true),
            tsumo(2, "9p"),
            dahai(2, "9p", true),
            tsumo(3, "9p"),
            dahai(3, "9p", true),
            // Turn 2: we draw W and discard it.
            tsumo(0, "W"),
            dahai(0, "W", true),
            tsumo(1, "9p"),
            dahai(1, "9p", true),
            tsumo(2, "9p"),
            dahai(2, "9p", true),
            tsumo(3, "9p"),
            dahai(3, "9p", true),
            MjaiEvent::EndKyoku,
            MjaiEvent::end_game(),
        ]
    }

    #[test]
    fn scans_own_discard_decisions_with_junme_and_14_tiles() {
        let res = scan_decisions(&small_game(), 0).unwrap();
        assert_eq!(res.num_players, 4);
        assert_eq!(res.decisions.len(), 2, "two own draw→discard windows");

        let first = &res.decisions[0];
        assert_eq!(first.junme, 1);
        assert_eq!(first.event_index, 2, "index of our first Tsumo");
        assert_eq!(first.material.tehai.len(), 14, "decision hand holds 14");
        assert_eq!(first.material.draw, "E");
        assert_eq!(first.material.actual, "dahai:E");
        assert!(
            !first.material.tehai.iter().any(|t| t == "?"),
            "observer seat sees real tiles"
        );
        assert_eq!(res.decisions[1].junme, 2);
        assert_eq!(res.decisions[1].material.actual, "dahai:W");
    }

    #[test]
    fn start_game_seat_injection_makes_hidden_hands_visible() {
        // Same stream but with an explicit perspective: no patching path.
        let with_id: Vec<MjaiEvent> = small_game()
            .into_iter()
            .map(|ev| match ev {
                MjaiEvent::StartGame { .. } => start_game(Some(0)),
                other => other,
            })
            .collect();
        let a = scan_decisions(&small_game(), 0).unwrap();
        let b = scan_decisions(&with_id, 0).unwrap();
        assert_eq!(a.decisions.len(), b.decisions.len());
        assert_eq!(a.decisions[0].material.tehai, b.decisions[0].material.tehai);
    }

    #[test]
    fn every_turn_has_candidates_context_and_engine_shanten() {
        let res = scan_decisions(&small_game(), 0).unwrap();
        for d in &res.decisions {
            assert!(
                !d.material.candidates.is_empty(),
                "junme {} must offer candidates",
                d.junme
            );
            let ctx = d.material.context.as_ref().expect("context present");
            assert_eq!(ctx.bakaze, "E");
            assert_eq!(ctx.kyoku, 1);
            assert_eq!(ctx.scores.len(), 4);
            for cand in &d.material.candidates {
                assert!(cand.label.starts_with("dahai:"));
                assert!(
                    (0.0..=1.0).contains(&cand.numbers.win_rate.unwrap_or(0.0)),
                    "win_rate is a 0..1 fraction, got {:?}",
                    cand.numbers.win_rate
                );
                if let Some(dir_) = cand.numbers.deal_in_rate {
                    assert!((0.0..=1.0).contains(&dir_), "deal_in_rate is 0..1");
                }
            }
            let prob_sum: f64 = d.material.candidates.iter().map(|c| c.prob).sum();
            assert!(
                (prob_sum - 1.0).abs() < 1e-9 || prob_sum == 0.0,
                "probs normalize to 1 (or all-zero): {prob_sum}"
            );
        }
    }

    #[test]
    fn coverage_key_keeps_key_turns_only() {
        let events = small_game();
        let key = build_review_material(
            &events,
            0,
            "01JTESTGAME0000000000000000",
            None,
            ReviewCoverage::Key,
        )
        .unwrap();
        let all = build_review_material(
            &events,
            0,
            "01JTESTGAME0000000000000000",
            None,
            ReviewCoverage::All,
        )
        .unwrap();
        let scan = scan_decisions(&events, 0).unwrap();
        let key_count = scan.decisions.iter().filter(|d| d.is_key).count();
        assert_eq!(all.turns.len(), scan.decisions.len());
        assert_eq!(key.turns.len(), key_count);
        for t in &key.turns {
            let dp = scan
                .decisions
                .iter()
                .find(|d| d.material.junme == t.junme)
                .unwrap();
            assert!(dp.is_key);
        }
    }

    #[test]
    fn draw_without_discard_produces_no_decision() {
        let events = vec![
            start_game(Some(0)),
            start_kyoku(),
            tsumo(0, "E"),
            // We tsumo-agari before discarding.
            MjaiEvent::Hora {
                actor: 0,
                target: 0,
                deltas: None,
                ura_markers: None,
            },
            MjaiEvent::EndKyoku,
            MjaiEvent::end_game(),
        ];
        let res = scan_decisions(&events, 0).unwrap();
        assert!(res.decisions.is_empty());
    }

    #[test]
    fn reach_between_draw_and_discard_marks_our_riichi() {
        let events = vec![
            start_game(Some(0)),
            start_kyoku(),
            tsumo(0, "E"),
            MjaiEvent::Reach {
                actor: 0,
                pai: None,
            },
            dahai(0, "E", false),
            MjaiEvent::EndKyoku,
            MjaiEvent::end_game(),
        ];
        let res = scan_decisions(&events, 0).unwrap();
        assert_eq!(res.decisions.len(), 1);
        assert!(
            res.decisions[0]
                .material
                .context
                .as_ref()
                .unwrap()
                .our_riichi
        );
    }

    #[test]
    fn key_rules_table() {
        let mk = |actual_rank: Option<usize>, opp_riichi: bool, shanten: i8, first_tenpai: bool| {
            KeyFacts {
                actual_rank,
                opp_riichi,
                best_shanten: shanten,
                first_tenpai,
                attack: Some("5s".into()),
                defence: Some("5s".into()),
                candidate_tiles: &["5s", "1z"],
            }
        };
        // Plain turn: actual is engine top choice, no threats.
        assert!(!is_key_turn(&mk(Some(0), false, 2, false)));
        // ① actual ranked 4th+.
        assert!(is_key_turn(&mk(Some(3), false, 2, false)));
        // ① actual not a maintain discard at all.
        assert!(is_key_turn(&mk(None, false, 2, false)));
        // ② opponent riichi while we're tenpai-ish.
        assert!(is_key_turn(&mk(Some(0), true, 1, false)));
        assert!(!is_key_turn(&mk(Some(0), true, 2, false)));
        // ③ first tenpai of the kyoku.
        assert!(is_key_turn(&mk(Some(0), false, 0, true)));
        assert!(!is_key_turn(&mk(Some(0), false, 0, false)));
        // ④ attack ≠ defence, both offered.
        let split = KeyFacts {
            actual_rank: Some(0),
            opp_riichi: false,
            best_shanten: 1,
            first_tenpai: false,
            attack: Some("5s".into()),
            defence: Some("1z".into()),
            candidate_tiles: &["5s", "1z"],
        };
        assert!(is_key_turn(&split));
        // ④ but defence pick not among offered candidates → not key.
        let split_hidden = KeyFacts {
            defence: Some("9p".into()),
            candidate_tiles: &["5s", "1z"],
            ..split
        };
        assert!(!is_key_turn(&split_hidden));
    }

    #[test]
    fn replay_snapshot_at_steps_through_the_stream() {
        let events = small_game();
        // Before the first draw: seat 0 holds 13 tiles, not a discard window.
        let pre = replay_snapshot_at(&events, 0, 1).unwrap().unwrap();
        assert!(!is_discard_window(&pre, 0));
        // Right after our first Tsumo (index 2): 14 tiles, discard window.
        let at_draw = replay_snapshot_at(&events, 0, 2).unwrap().unwrap();
        assert!(is_discard_window(&at_draw, 0));
        assert_eq!(at_draw.players[0].tehai.len(), 14);
        // Injection: same stream, untagged StartGame, still sees real tiles.
        assert!(!at_draw.players[0].tehai.iter().any(|t| t == "?"));
        // Out of range is an error, not a panic.
        assert!(replay_snapshot_at(&events, 0, events.len()).is_err());
        // Outline carries decision indices and key flags.
        let scan = scan_decisions(&events, 0).unwrap();
        let outline = scan.to_outline(0, events.len());
        assert_eq!(outline.total_events as usize, events.len());
        assert_eq!(outline.decisions.len(), 2);
        assert_eq!(outline.decisions[0].event_index, 2);
        assert_eq!(outline.decisions[0].actual, "E");
        assert_eq!(outline.decisions[1].actual, "W");
    }
}
