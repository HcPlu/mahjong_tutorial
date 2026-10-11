//! Live coach deep-dive (M2, D-0008): the backend half of the in-game
//! "explain this decision" button, plus its automatic sibling (D-0014).
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
//! # Auto mode (D-0014)
//!
//! With `[tutorial.coach] auto_policy = "all"` the same decision point
//! is also queued for background generation **at draw time** — the
//! request races the player's discard window (measured ~3-8 s LLM
//! latency vs a 10-20 s window), so the explanation usually lands
//! before the decision. A single serial worker (`auto_explain_worker`)
//! drains the queue: one in-flight request at most, every gate re-read
//! from the live config per job (flipping consent off mid-game stops
//! outbound traffic immediately), cache-first, results pushed on the
//! `TutorialLiveBus`. No streaming: a `TurnDoc` is one JSON object, so
//! partial bodies could not be rendered anyway.
//!
//! Boundary note: `crate::bot::BotResponse` is the public wire type of
//! the `BotResponseBus` (it also travels to the frontend via IPC). It is
//! the only bot symbol this module names — no bot internals are called.

use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::{broadcast, mpsc, Mutex as AsyncMutex, RwLock as AsyncRwLock};
use tracing::{debug, warn};

use crate::analysis::snapshot_adapter::to_player_info;
use crate::analysis::{analyze, AnalysisResult, DiscardCandidate};
use crate::bot::BotResponse;
use crate::config::{AppConfig, AutoPolicy, LlmConfig};
use crate::event_bus::{TrackedEvent, TutorialLiveBus};
use crate::game_state::snapshot::GameStateSnapshot;
use crate::game_state::tracker::GameTracker;
use crate::schema::MjaiEvent;
use crate::tutorial::assemble::{engine_numbers, is_discard_window, turn_context};
use crate::tutorial::context::TutorialContext;
use crate::tutorial::material::{CandidateNumbers, LiveTurnDoc, TurnMaterial, TutorialLiveEvent};
use crate::tutorial::provider::TutorialProvider;

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

/// One queued auto-analysis job (D-0014): the material snapshot for a
/// decision point, handed from the drive loop to the serial worker.
pub struct AutoJob {
    pub session_key: String,
    pub material: TurnMaterial,
}

/// Drive the live coach over the two buses until they close. Spawned in
/// `lib.rs` inside the Tauri runtime.
///
/// `auto_tx` receives a job per decision point while
/// `coach.auto_policy != Off` (the policy is re-read per event, so the
/// Settings toggle takes effect without a relaunch). The queue is
/// bounded; a full queue means the worker is multiple turns behind —
/// only realistic during a reconnect burst, where stale restore turns
/// are exactly the ones worth dropping.
///
/// The same decision points also feed the D-0017 bot-answer recorder:
/// every bot response that lands on our discard window is appended to
/// the session's `LiveBotDoc` and flushed to the store at session end
/// (`StartGame`/`EndGame`). Pure bookkeeping — failures are logged,
/// never propagated into the coach path.
pub async fn drive_loop(
    mut post_rx: broadcast::Receiver<TrackedEvent>,
    mut bot_rx: broadcast::Receiver<BotResponse>,
    tracker: Arc<AsyncMutex<GameTracker>>,
    coach: LiveCoachHandle,
    config: Arc<AsyncRwLock<AppConfig>>,
    auto_tx: mpsc::Sender<AutoJob>,
    ctx: Arc<TutorialContext>,
) {
    debug!("live coach subscribed to post-tracker + bot-response buses");
    let mut session_key = String::new();
    let mut seat: Option<u8> = None;
    let mut our_draws: u32 = 0;
    let mut botlive = crate::tutorial::botlive::BotLiveRecorder::default();

    loop {
        tokio::select! {
            ev = post_rx.recv() => {
                match ev {
                    Ok(TrackedEvent { event, .. }) => match &event {
                        MjaiEvent::StartGame { id, num_players, .. } => {
                            flush_botlive(&ctx, &mut botlive);
                            seat = *id;
                            our_draws = 0;
                            session_key = new_session_key(*id);
                            // Which bot is answering this game — recorded
                            // once per session so the replay page can say
                            // "the bot that played then" precisely.
                            let bot_name = crate::tutorial::botlive::active_bot_name(
                                &*config.read().await,
                                *num_players,
                            );
                            if let Some(seat) = seat {
                                botlive.start(&session_key, seat, &bot_name);
                            }
                        }
                        MjaiEvent::EndGame { .. } => {
                            flush_botlive(&ctx, &mut botlive);
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
                            botlive.record(crate::tutorial::botlive::classify_response(
                                material.junme,
                                &resp,
                            ));
                            let _ = seat; // seat already bound above
                            // `coach.enabled` is the master switch — auto
                            // generation must not spend money while the
                            // tile (its only renderer) is switched off.
                            let coach_on = {
                                let coach_cfg =
                                    &config.read().await.tutorial.coach;
                                coach_cfg.enabled && coach_cfg.auto_policy != AutoPolicy::Off
                            };
                            if coach_on {
                                let job = AutoJob {
                                    session_key: session_key.clone(),
                                    material: material.clone(),
                                };
                                if let Err(mpsc::error::TrySendError::Full(_)) =
                                    auto_tx.try_send(job)
                                {
                                    debug!(
                                        "live auto queue full — dropping this turn's \
                                         auto analysis (worker is behind)"
                                    );
                                }
                            }
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

/// Persist the recorder's current session, if it has any turns.
/// Recording must never break the coach: store errors are warnings.
fn flush_botlive(
    ctx: &Arc<TutorialContext>,
    botlive: &mut crate::tutorial::botlive::BotLiveRecorder,
) {
    if let Some(doc) = botlive.finish() {
        if let Err(e) = ctx.store.save_botlive(&doc) {
            warn!("failed to save live bot doc for {}: {e:#}", doc.session_key);
        }
    }
}

/// Serial background generator for auto live-turn deep-dives (D-0014).
/// One job at a time — an LLM call takes seconds, and fanning out into
/// parallel requests would only stack rate limits. Every gate is re-read
/// from the live config **per job**, so flipping `auto_policy` or
/// consent off mid-game stops the very next outbound call.
///
/// Per job: policy + consent gate → cache lookup (a hit still emits the
/// event, so a frontend that missed the original — reload, lagged
/// receiver — catches up) → provider call via `factory` → trust parse →
/// save → `TutorialLiveBus` broadcast. Any failure is logged and
/// dropped, never retried: the next draw is a fresh chance.
pub async fn auto_explain_worker(
    mut rx: mpsc::Receiver<AutoJob>,
    ctx: Arc<TutorialContext>,
    config: Arc<AsyncRwLock<AppConfig>>,
    bus: TutorialLiveBus,
    factory: impl Fn(&LlmConfig) -> anyhow::Result<Arc<dyn TutorialProvider>>,
) {
    debug!("live auto explain worker started");
    while let Some(job) = rx.recv().await {
        let cfg = config.read().await.tutorial.clone();
        // Consume-time re-check: the policy (or the coach master switch)
        // may have been turned off while this job sat in the queue.
        if !cfg.coach.enabled || cfg.coach.auto_policy == AutoPolicy::Off {
            debug!("live auto skipped (coach off at consume time)");
            continue;
        }
        if !crate::tutorial::upload::outbound_allowed(&cfg) {
            debug!("live auto skipped (outbound not allowed)");
            continue;
        }
        let key = format!("{}-t{}", job.session_key, job.material.junme);
        // Cache before request (invariant 3) — same key the manual
        // button uses, so an auto-generated turn is free to click.
        match ctx.store.load_turn_doc(&key) {
            Ok(Some(cached)) => {
                emit_live_turn(&bus, &job.session_key, &cached);
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                warn!("live auto cache read failed for {key}: {e:#} — regenerating");
            }
        }
        let provider = match factory(&cfg.llm) {
            Ok(p) => p,
            Err(e) => {
                warn!("live auto provider build failed: {e:#}");
                continue;
            }
        };
        match crate::tutorial::pipeline::generate_live_turn(
            &ctx.store,
            provider.as_ref(),
            &job.material,
            &key,
        )
        .await
        {
            Ok(live_doc) => emit_live_turn(&bus, &job.session_key, &live_doc),
            Err(e) => {
                // Logged, not fatal: the manual button and the next turn
                // both remain available.
                warn!("live auto generation failed for {key}: {e:#}");
            }
        }
    }
    debug!("live auto explain worker stopped (queue closed)");
}

/// Broadcast one finished turn. A closed bus (no subscribers) is fine —
/// the doc is on disk either way.
fn emit_live_turn(bus: &TutorialLiveBus, session_key: &str, doc: &LiveTurnDoc) {
    let junme = doc.doc.junme;
    let _ = bus.send(TutorialLiveEvent {
        session_key: session_key.to_string(),
        junme,
        doc: doc.doc.clone(),
        model: doc.model.clone(),
    });
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ----- worker test harness -----

    /// Scripted provider: pops one canned reply per call, counts calls.
    /// Shared behind an Arc so the factory (which builds a fresh provider
    /// per job, like production does) still sees one counter.
    struct FakeState {
        replies: Mutex<std::collections::VecDeque<String>>,
        calls: AtomicUsize,
    }

    struct FakeProvider {
        shared: Arc<FakeState>,
    }

    #[async_trait::async_trait]
    impl TutorialProvider for FakeProvider {
        fn name(&self) -> &'static str {
            "fake"
        }

        async fn complete(
            &self,
            _req: crate::tutorial::provider::ChatRequest,
        ) -> anyhow::Result<crate::tutorial::provider::ChatResponse> {
            self.shared.calls.fetch_add(1, Ordering::SeqCst);
            let content = self
                .shared
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| valid_turn_json("备用讲解。"));
            Ok(crate::tutorial::provider::ChatResponse {
                content,
                model: Some("fake-model".into()),
                usage: None,
            })
        }
    }

    fn valid_turn_json(text: &str) -> String {
        json!({
            "junme": 0,
            "recommended": "dahai:E",
            "numbers": {},
            "explanation_zh": text,
            "headline": "测试讲解",
        })
        .to_string()
    }

    fn app_cfg(policy: AutoPolicy, consent: bool, configured: bool) -> AppConfig {
        let mut c = AppConfig::default();
        c.tutorial.enabled = true;
        c.tutorial.upload_consent = consent;
        if configured {
            c.tutorial.llm = LlmConfig {
                base_url: "http://127.0.0.1:9/v1".into(),
                api_key: "k".into(),
                model: "m".into(),
                ..Default::default()
            };
        }
        c.tutorial.coach.enabled = true;
        c.tutorial.coach.auto_policy = policy;
        c
    }

    fn fake_material(junme: u32) -> TurnMaterial {
        TurnMaterial {
            junme,
            tehai: vec![
                "1m", "2m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "S", "E",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            draw: "E".into(),
            actual: "dahai:E".into(),
            candidates: vec![CandidateNumbers {
                label: "dahai:E".into(),
                prob: 0.7,
                numbers: Default::default(),
            }],
            context: None,
        }
    }

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!(
            "akagi-tutorial-live-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn test_ctx(tag: &str) -> Arc<TutorialContext> {
        Arc::new(
            TutorialContext::new(temp_root(tag), LiveCoachHandle::default())
                .expect("tutorial context"),
        )
    }

    /// Run the worker over `jobs`, collect everything it broadcast.
    /// Returns (events, provider_call_count).
    async fn run_worker(
        cfg: AppConfig,
        ctx: Arc<TutorialContext>,
        jobs: Vec<AutoJob>,
        shared: Arc<FakeState>,
    ) -> (Vec<TutorialLiveEvent>, usize) {
        let (tx, rx) = mpsc::channel(16);
        for j in jobs {
            tx.send(j).await.unwrap();
        }
        drop(tx);
        let (bus, _keeper) = broadcast::channel::<TutorialLiveEvent>(64);
        let mut sub = bus.subscribe();
        let factory_shared = shared.clone();
        auto_explain_worker(
            rx,
            ctx,
            Arc::new(AsyncRwLock::new(cfg)),
            bus.clone(),
            move |_cfg| {
                Ok(Arc::new(FakeProvider {
                    shared: factory_shared.clone(),
                }) as Arc<dyn TutorialProvider>)
            },
        )
        .await;
        let mut events = Vec::new();
        while let Ok(ev) = sub.try_recv() {
            events.push(ev);
        }
        (events, shared.calls.load(Ordering::SeqCst))
    }

    fn job(session: &str, junme: u32) -> AutoJob {
        AutoJob {
            session_key: session.into(),
            material: fake_material(junme),
        }
    }

    #[tokio::test]
    async fn worker_skips_when_policy_consent_or_endpoint_missing() {
        // Master-switch-off variant: coach tile disabled → no spend even
        // with the policy on (the tile is the auto docs' only renderer).
        let mut coach_off = app_cfg(AutoPolicy::All, true, true);
        coach_off.tutorial.coach.enabled = false;
        for (tag, cfg) in [
            ("policy-off", app_cfg(AutoPolicy::Off, true, true)),
            ("coach-off", coach_off),
            ("consent-off", app_cfg(AutoPolicy::All, false, true)),
            ("no-endpoint", app_cfg(AutoPolicy::All, true, false)),
        ] {
            let shared = Arc::new(FakeState {
                replies: Mutex::new(std::collections::VecDeque::new()),
                calls: AtomicUsize::new(0),
            });
            let (events, calls) = run_worker(
                cfg,
                test_ctx(tag),
                vec![job("20260101-000000-0", 3)],
                shared,
            )
            .await;
            assert_eq!(calls, 0, "{tag}: gated jobs must not reach the provider");
            assert!(events.is_empty(), "{tag}: nothing to broadcast");
        }
    }

    #[tokio::test]
    async fn worker_cache_hit_short_circuits_and_still_emits() {
        let ctx = test_ctx("cache-hit");
        let key = "20260101-000000-0-t3";
        ctx.store
            .save_turn_doc(&LiveTurnDoc {
                key: key.into(),
                doc: crate::tutorial::material::TurnDoc {
                    junme: 3,
                    recommended: "dahai:E".into(),
                    explanation_zh: "缓存讲解。".into(),
                    ..Default::default()
                },
                model: Some("old-model".into()),
                created_at: "2026-01-01T00:00:00Z".into(),
            })
            .unwrap();
        let shared = Arc::new(FakeState {
            replies: Mutex::new(std::collections::VecDeque::new()),
            calls: AtomicUsize::new(0),
        });
        let (events, calls) = run_worker(
            app_cfg(AutoPolicy::All, true, true),
            ctx,
            vec![job("20260101-000000-0", 3)],
            shared,
        )
        .await;
        assert_eq!(calls, 0, "cache hit must not call the provider");
        assert_eq!(events.len(), 1, "cache hit still notifies the frontend");
        assert_eq!(events[0].junme, 3);
        assert_eq!(events[0].doc.explanation_zh, "缓存讲解。");
        assert_eq!(events[0].model.as_deref(), Some("old-model"));
    }

    #[tokio::test]
    async fn worker_generates_saves_and_emits() {
        let ctx = test_ctx("generate");
        let shared = Arc::new(FakeState {
            replies: Mutex::new(std::collections::VecDeque::from(vec![valid_turn_json(
                "先切孤张字牌，保留两面搭子。",
            )])),
            calls: AtomicUsize::new(0),
        });
        let (events, calls) = run_worker(
            app_cfg(AutoPolicy::All, true, true),
            ctx.clone(),
            vec![job("20260101-000000-0", 7)],
            shared,
        )
        .await;
        assert_eq!(calls, 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].session_key, "20260101-000000-0");
        assert_eq!(events[0].junme, 7, "junme is stamped from the material");
        assert_eq!(events[0].doc.explanation_zh, "先切孤张字牌，保留两面搭子。");
        // Same key the manual button reads → the click becomes free.
        let cached = ctx
            .store
            .load_turn_doc("20260101-000000-0-t7")
            .expect("load")
            .expect("saved to disk");
        assert_eq!(cached.doc.junme, 7);
    }

    #[tokio::test]
    async fn worker_degrades_silently_on_garbage_reply() {
        let ctx = test_ctx("garbage");
        let shared = Arc::new(FakeState {
            replies: Mutex::new(std::collections::VecDeque::from(vec![
                "完全不是 JSON 的回复".into(),
            ])),
            calls: AtomicUsize::new(0),
        });
        let (events, calls) = run_worker(
            app_cfg(AutoPolicy::All, true, true),
            ctx.clone(),
            vec![job("20260101-000000-0", 2)],
            shared,
        )
        .await;
        assert_eq!(calls, 1, "the call was made once");
        assert!(events.is_empty(), "an unparseable reply broadcasts nothing");
        // And nothing was persisted under that key.
        assert!(ctx
            .store
            .load_turn_doc("20260101-000000-0-t2")
            .unwrap()
            .is_none());
    }

    // ----- drive_loop trigger wiring -----

    #[tokio::test]
    async fn drive_loop_queues_only_when_policy_on() {
        use crate::game_state::tracker;

        let start_game = MjaiEvent::StartGame {
            names: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            kyoku_first: None,
            aka_flag: None,
            id: Some(0),
            num_players: 4,
            game_meta: None,
        };
        let ours: Vec<String> = [
            "1m", "2m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "S",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let hidden: Vec<String> = (0..13).map(|_| "?".into()).collect();
        let start_kyoku = MjaiEvent::StartKyoku {
            bakaze: "E".into(),
            dora_marker: "2m".into(),
            kyoku: 1,
            honba: 0,
            kyotaku: 0,
            oya: 0,
            scores: vec![25_000; 4],
            tehais: vec![ours, hidden.clone(), hidden.clone(), hidden],
            num_players: 4,
        };
        let tsumo_e = MjaiEvent::Tsumo {
            actor: 0,
            pai: "E".into(),
        };

        // Pre-feed the shared tracker exactly what the real tracker loop
        // would have applied when the bot answers.
        let tracker = tracker::new_handle();
        for ev in [&start_game, &start_kyoku, &tsumo_e] {
            tracker.lock().await.handle(ev).expect("feed tracker");
        }

        let (post_tx, post_rx) = broadcast::channel(16);
        let (bot_tx, bot_rx) = broadcast::channel(16);
        let cfg = Arc::new(AsyncRwLock::new(app_cfg(AutoPolicy::Off, true, true)));
        let (auto_tx, mut auto_rx) = mpsc::channel(4);
        let coach = LiveCoachHandle::default();
        let task = tokio::spawn(drive_loop(
            post_rx,
            bot_rx,
            tracker.clone(),
            coach.clone(),
            cfg.clone(),
            auto_tx,
            test_ctx("drive-loop"),
        ));

        let tracked = |ev: MjaiEvent| TrackedEvent {
            event: ev,
            can_act: None,
        };
        for ev in [&start_game, &start_kyoku, &tsumo_e] {
            post_tx.send(tracked(ev.clone())).unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        // Policy off: the decision material exists but must NOT queue.
        bot_tx.send(bot_dahai("E", None)).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(
            auto_rx.try_recv().is_err(),
            "policy off must not queue auto jobs"
        );
        assert!(coach.current().is_some(), "slot is filled regardless");

        // Flip the policy; the very next response queues a job.
        cfg.write().await.tutorial.coach.auto_policy = AutoPolicy::All;
        bot_tx.send(bot_dahai("E", None)).unwrap();
        let job = tokio::time::timeout(std::time::Duration::from_secs(2), auto_rx.recv())
            .await
            .expect("job queued in time")
            .expect("channel alive");
        assert_eq!(job.material.junme, 1, "junme counted from our draws");
        assert!(
            job.session_key.ends_with("-0"),
            "session key carries the seat: {}",
            job.session_key
        );

        task.abort();
    }

    // ----- existing snapshot-level tests below -----

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
