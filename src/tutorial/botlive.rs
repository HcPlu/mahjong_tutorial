//! Live bot-answer recording (D-0017): while a game is being played,
//! every bot answer that lands on our discard window is captured — the
//! action plus its `meta.show` distribution — so the replay page can
//! show **what the bot that actually played this game chose**, next to
//! the regenerated reference from whatever bot is configured at review
//! time (the two may differ: different bot, weights, or temperature).
//!
//! Recording happens inside `live::drive_loop` at the same decision
//! points the coach materialises (session key, junme and the
//! discard-window gate are already resolved there). One `LiveBotDoc`
//! per live session lands in `<root>/botlive/<session_key>.json` —
//! pure local data, same class as the offline bot reference: no LLM,
//! no consent gate.
//!
//! Joining a session file to a finalised history record is by
//! `(seat, started_at)` time window, not a foreign key: both the
//! history recorder and the live recorder stamp `Utc::now()` when the
//! same `StartGame` reaches them, so the drift is milliseconds and the
//! matcher allows a minute. A mid-game reconnect (page reload, dead
//! clicks) splits one logical game into several live sessions — the
//! matcher merges them by junme, later session winning, because the
//! reconnect's restore replay fast-forwards the live junme count back
//! to the true value.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::bot::BotResponse;
use crate::schema::history::GameRecord;
use crate::schema::MjaiEvent;
use crate::tutorial::material::{BotRefDoc, BotRefPick, BotRefSource, BotRefTurn};

/// One recorded live session, persisted under `botlive/<session_key>.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveBotDoc {
    /// Session key (`YYYYMMDD-HHMMSS-seat`) — the file stem, and the
    /// same key live-coach explanations use.
    pub session_key: String,
    /// The seat we were playing (`StartGame.id`).
    pub seat: u8,
    /// Bot name active when the session started (from config, by
    /// player count) — what actually produced these answers.
    pub bot: String,
    /// RFC 3339 UTC — when `StartGame` reached the recorder; joins to
    /// the history record's `started_at`.
    pub started_at: String,
    /// RFC 3339 UTC — when the doc was flushed to disk.
    pub created_at: String,
    pub turns: Vec<BotRefTurn>,
}

/// Convert one bot answer into the persisted tutorial shape. Shared by
/// the live recorder (this module, via `live::drive_loop`) and the ipc
/// replay orchestrator — both sides meet the same bot wire format.
///
/// `classify_reaction` is `crate::bot`'s public API (AGENTS §6.4);
/// everything below it stays tutorial-side types.
pub fn to_botref_turn(
    junme: u32,
    action: &MjaiEvent,
    source: crate::bot::replay::ReplaySource,
    picks: Vec<crate::bot::replay::ReplayPick>,
) -> BotRefTurn {
    BotRefTurn {
        junme,
        action: action_label(action),
        picks: picks
            .into_iter()
            .map(|p| BotRefPick {
                tile: p.tile,
                label: p.label,
                prob: p.prob,
                chosen: p.chosen,
            })
            .collect(),
        source: Some(map_source(source)),
    }
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
fn map_source(source: crate::bot::replay::ReplaySource) -> BotRefSource {
    match source {
        crate::bot::replay::ReplaySource::Model => BotRefSource::Model,
        crate::bot::replay::ReplaySource::RuleForced => BotRefSource::RuleForced,
        crate::bot::replay::ReplaySource::RuleOverridden => BotRefSource::RuleOverridden,
        crate::bot::replay::ReplaySource::NoAnswer => BotRefSource::NoAnswer,
    }
}

/// Session-scoped buffer the drive loop fills. A fresh session resets
/// it; `finish` yields the doc for the store (empty sessions are
/// dropped — a game with zero bot answers writes nothing).
#[derive(Default)]
pub struct BotLiveRecorder {
    doc: Option<LiveBotDoc>,
}

impl BotLiveRecorder {
    /// Open a new session, replacing any previous one (the caller
    /// flushes the old doc first).
    pub fn start(&mut self, session_key: &str, seat: u8, bot: &str) {
        self.doc = Some(LiveBotDoc {
            session_key: session_key.to_string(),
            seat,
            bot: bot.to_string(),
            started_at: Utc::now().to_rfc3339(),
            created_at: String::new(),
            turns: Vec::new(),
        });
    }

    /// Record the answer for one decision point. The same junme can be
    /// answered twice (bot retry at an unchanged window) — the later
    /// answer wins.
    pub fn record(&mut self, turn: BotRefTurn) {
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        match doc.turns.iter_mut().find(|t| t.junme == turn.junme) {
            Some(slot) => *slot = turn,
            None => doc.turns.push(turn),
        }
    }

    /// Close the session and take its doc (stamping `created_at`), if
    /// it has any turns.
    pub fn finish(&mut self) -> Option<LiveBotDoc> {
        let mut doc = self.doc.take()?;
        if doc.turns.is_empty() {
            return None;
        }
        doc.created_at = Utc::now().to_rfc3339();
        Some(doc)
    }
}

/// How far a live session's `started_at` may sit from the history
/// record's window and still count as the same game.
const MATCH_TOLERANCE_SECS: i64 = 60;

/// Find the live session(s) belonging to one finalised game and merge
/// them into a single reference doc. Match: same seat, `started_at`
/// within `[record.started_at − tol, record.ended_at + tol]`. Merge:
/// by junme, later-started session wins on collision (reconnect
/// restore replays realign the junme). `None` when nothing matches.
pub fn match_live_bot(record: &GameRecord, docs: Vec<LiveBotDoc>) -> Option<BotRefDoc> {
    let seat = record.our_seat?;
    let tol = chrono::Duration::seconds(MATCH_TOLERANCE_SECS);
    let mut matched: Vec<LiveBotDoc> = docs
        .into_iter()
        .filter_map(|d| {
            let started = DateTime::<Utc>::from(DateTime::parse_from_rfc3339(&d.started_at).ok()?);
            let in_window = started + tol >= record.started_at && started - tol <= record.ended_at;
            (d.seat == seat && in_window).then_some(d)
        })
        .collect();
    if matched.is_empty() {
        return None;
    }
    matched.sort_by(|a, b| a.started_at.cmp(&b.started_at));

    let mut merged = std::collections::BTreeMap::new();
    for doc in &matched {
        for turn in &doc.turns {
            merged.insert(turn.junme, turn.clone());
        }
    }
    // Provenance of the merged doc: the bot of the session that is
    // still standing at game end (the last one to start).
    let last = matched.last().expect("matched is non-empty");
    Some(BotRefDoc {
        game_id: record.id.clone(),
        bot: last.bot.clone(),
        created_at: last.created_at.clone(),
        turns: merged.into_values().collect(),
    })
}

/// Convenience wrapper for the recorder side of `live::drive_loop`:
/// the bot configured for the game's player count (empty when unset —
/// recorded as an empty name; the doc is still meaningful as "some
/// bot answered").
pub fn active_bot_name(cfg: &crate::config::AppConfig, num_players: u8) -> String {
    if num_players == 3 {
        cfg.bot.active_3p.clone()
    } else {
        cfg.bot.active_4p.clone()
    }
}

/// Classify one live response for tests/callers that already hold a
/// `BotResponse`.
pub fn classify_response(junme: u32, resp: &BotResponse) -> BotRefTurn {
    let (source, picks) = crate::bot::replay::classify_reaction(&resp.action, &resp.meta);
    to_botref_turn(junme, &resp.action, source, picks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(started: &str, ended: &str, seat: Option<u8>) -> GameRecord {
        GameRecord {
            id: "01JTESTGAME0000000000000000".into(),
            started_at: DateTime::parse_from_rfc3339(started)
                .unwrap()
                .with_timezone(&Utc),
            ended_at: DateTime::parse_from_rfc3339(ended)
                .unwrap()
                .with_timezone(&Utc),
            platform: crate::schema::Platform::Majsoul,
            num_players: 4,
            kyoku_mode: crate::schema::KyokuMode::EastSouth,
            names: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            our_seat: seat,
            final_scores: vec![25_000; 4],
            final_ranks: vec![1, 2, 3, 4],
            our_rank: None,
            our_delta: None,
            stats: Default::default(),
            match_info: None,
            log_path: "games/x.mjai.jsonl".into(),
        }
    }

    fn doc(session: &str, seat: u8, started: &str, turns: Vec<(u32, &str)>) -> LiveBotDoc {
        LiveBotDoc {
            session_key: session.into(),
            seat,
            bot: "mortal".into(),
            started_at: started.into(),
            created_at: started.into(),
            turns: turns
                .into_iter()
                .map(|(junme, action)| BotRefTurn {
                    junme,
                    action: action.into(),
                    ..Default::default()
                })
                .collect(),
        }
    }

    #[test]
    fn recorder_drops_empty_sessions_and_replaces_by_junme() {
        let mut rec = BotLiveRecorder::default();
        // Nothing recorded before a session starts.
        rec.record(BotRefTurn {
            junme: 1,
            ..Default::default()
        });
        assert!(rec.finish().is_none());

        rec.start("20260101-120000-0", 0, "mortal");
        assert!(rec.finish().is_none(), "empty session writes nothing");

        rec.start("20260101-120100-0", 0, "mortal");
        rec.record(BotRefTurn {
            junme: 1,
            action: "dahai:E".into(),
            ..Default::default()
        });
        rec.record(BotRefTurn {
            junme: 1,
            action: "dahai:W".into(),
            ..Default::default()
        });
        rec.record(BotRefTurn {
            junme: 2,
            action: "reach".into(),
            ..Default::default()
        });
        let doc = rec.finish().unwrap();
        assert_eq!(doc.turns.len(), 2);
        assert_eq!(doc.turns[0].action, "dahai:W", "later answer wins");
        assert_eq!(doc.turns[1].junme, 2);
        assert!(rec.finish().is_none(), "finished twice yields nothing");
    }

    #[test]
    fn matcher_joins_by_seat_and_time_window() {
        let rec = record("2026-01-01T12:00:30Z", "2026-01-01T12:20:00Z", Some(1));
        // Wrong seat, and a game far outside the window: ignored.
        let docs = vec![
            doc("a", 2, "2026-01-01T12:00:31Z", vec![(1, "dahai:E")]),
            doc("b", 1, "2025-12-01T12:00:31Z", vec![(1, "dahai:E")]),
        ];
        assert!(match_live_bot(&rec, docs).is_none());

        let docs = vec![
            doc("a", 2, "2026-01-01T12:00:31Z", vec![(1, "dahai:E")]),
            doc(
                "ok",
                1,
                "2026-01-01T12:00:29Z",
                vec![(1, "dahai:E"), (2, "dahai:S")],
            ),
        ];
        let out = match_live_bot(&rec, docs).unwrap();
        assert_eq!(out.game_id, rec.id);
        assert_eq!(out.turns.len(), 2);
    }

    #[test]
    fn matcher_merges_reconnect_sessions_with_later_winning() {
        let rec = record("2026-01-01T12:00:30Z", "2026-01-01T12:20:00Z", Some(0));
        // Pre-reload session covered junme 1-5; the reload's restore
        // replay re-answers 1-5 identically-ish and adds 6-8.
        let docs = vec![
            doc(
                "first",
                0,
                "2026-01-01T12:00:31Z",
                vec![(1, "dahai:E"), (2, "dahai:S"), (5, "dahai:W")],
            ),
            doc(
                "second",
                0,
                "2026-01-01T12:10:00Z",
                vec![(2, "dahai:P"), (6, "reach"), (7, "dahai:1m")],
            ),
        ];
        let out = match_live_bot(&rec, docs).unwrap();
        let by_junme: Vec<(u32, String)> = out
            .turns
            .iter()
            .map(|t| (t.junme, t.action.clone()))
            .collect();
        assert_eq!(
            by_junme,
            vec![
                (1, "dahai:E".to_string()),
                (2, "dahai:P".to_string()), // later session wins the collision
                (5, "dahai:W".to_string()),
                (6, "reach".to_string()),
                (7, "dahai:1m".to_string()),
            ]
        );
    }

    #[test]
    fn response_classification_maps_show_card() {
        let resp = BotResponse {
            action: MjaiEvent::Dahai {
                actor: 0,
                pai: "5s".into(),
                tsumogiri: false,
            },
            meta: Some(serde_json::json!({
                "show": { "items": [
                    { "label": "Discard", "pais": ["5s"], "prob": 0.8 },
                    { "label": "Discard", "pais": ["1z"], "prob": 0.2 },
                ]},
                "is_greedy": true,
            })),
        };
        let turn = classify_response(3, &resp);
        assert_eq!(turn.junme, 3);
        assert_eq!(turn.action, "dahai:5s");
        assert_eq!(turn.source, Some(BotRefSource::Model));
        assert_eq!(turn.picks.len(), 2);
        assert!(turn.picks[0].chosen);
        assert!(!turn.picks[1].chosen);

        // No meta at all on a discard = quick_eval's forced discard.
        let forced = BotResponse {
            action: MjaiEvent::Dahai {
                actor: 0,
                pai: "E".into(),
                tsumogiri: true,
            },
            meta: None,
        };
        let turn = classify_response(4, &forced);
        assert_eq!(turn.source, Some(BotRefSource::RuleForced));
        assert!(turn.picks.is_empty());
    }
}
