//! Headless driver for the tutorial review pipeline (fork-only dev tool).
//!
//! Runs exactly what the UI's "生成讲解" runs — same config file, same
//! history store, same tutorial cache directory as the running app — but
//! from the command line, so the pipeline can be exercised and debugged
//! without touching the GUI:
//!
//!   cargo run --bin tutorial-debug -- --game latest
//!   cargo run --bin tutorial-debug -- --game latest --coverage all --refresh
//!   cargo run --bin tutorial-debug -- --game 01JXXXX... --json
//!
//! The LLM key is read the same way as in the app: config
//! `[tutorial.llm] api_key`, overridden by the TUTORIAL_LLM_API_KEY env
//! var. Reviews generated here land in the shared cache, so opening the
//! same game in the UI shows them without another call.

use std::path::Path;

use akagi::config::load_config;
use akagi::history::HistoryStore;
use akagi::tutorial::material::ReviewCoverage;
use akagi::tutorial::pipeline::generate_review;
use akagi::tutorial::store::TutorialStore;
use akagi::util::resolve_dir;

#[tokio::main]
async fn main() {
    let mut game: String = "latest".into();
    let mut coverage = ReviewCoverage::Key;
    let mut refresh = false;
    let mut json = false;
    let mut inspect: Option<u32> = None;
    let mut botref = false;
    let mut dump = false;
    let mut live_sim = false;
    let mut fake = false;
    let mut no_consent = false;
    let mut use_botref = false;
    let mut effort: Option<String> = None;
    let mut limit: Option<usize> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next().unwrap_or_else(|| usage("--game needs a value")),
            "--coverage" => {
                coverage = match args.next().as_deref() {
                    Some("key") => ReviewCoverage::Key,
                    Some("all") => ReviewCoverage::All,
                    other => usage(&format!("--coverage expects key|all, got {other:?}")),
                }
            }
            "--refresh" => refresh = true,
            "--botref" => botref = true,
            "--dump" => dump = true,
            "--live-sim" => live_sim = true,
            "--fake" => fake = true,
            "--no-consent" => no_consent = true,
            "--use-botref" => use_botref = true,
            "--effort" => {
                effort = Some(
                    args.next()
                        .unwrap_or_else(|| usage("--effort needs low|medium|high|none")),
                );
            }
            "--limit" => {
                limit = match args.next().map(|v| v.parse::<usize>()) {
                    Some(Ok(n)) if n > 0 => Some(n),
                    _ => usage("--limit needs a positive number"),
                }
            }
            "--json" => json = true,
            "--inspect" => {
                inspect = match args.next().map(|v| v.parse::<u32>()) {
                    Some(Ok(n)) => Some(n),
                    _ => usage("--inspect needs a junme number"),
                };
            }
            other => usage(&format!("unknown argument {other:?}")),
        }
    }

    let (cfg, _path) = load_config(None);

    let history = HistoryStore::new(resolve_dir(Path::new("./history")))
        .expect("open history store (run next to the app so ./history resolves)");
    let store =
        TutorialStore::new(resolve_dir(Path::new("./tutorial"))).expect("open tutorial store");

    let records = history.read_all().expect("read history index");
    if records.is_empty() {
        eprintln!("no recorded games in the history store");
        std::process::exit(2);
    }
    // The index is append-order (oldest first); ULIDs sort by time.
    let record = if game == "latest" {
        records.iter().max_by_key(|r| r.id.clone()).unwrap()
    } else {
        records
            .iter()
            .find(|r| r.id == game)
            .unwrap_or_else(|| usage(&format!("game {game:?} not found")))
    };
    let seat = record.our_seat.unwrap_or(0);
    println!(
        "# game {} ({}p, seat {}, rank {:?})",
        record.id, record.num_players, seat, record.our_rank,
    );

    if refresh {
        store.delete_review(&record.id).ok();
        println!("# cache cleared (--refresh)");
    }

    let events = history
        .get_events(&record.id)
        .expect("read event stream")
        .unwrap_or_else(|| usage("game has no event stream"));

    // --inspect <junme>: dump the engine's raw candidate table for one
    // decision point (what the UI's candidate chart renders). No network.
    if let Some(junme) = inspect {
        let scan = akagi::tutorial::scan_decisions(&events, seat).expect("scan");
        let dp = scan
            .decisions
            .iter()
            .find(|d| d.junme == junme)
            .unwrap_or_else(|| {
                usage(&format!(
                    "junme {junme} not found (game has {} decisions; max junme {})",
                    scan.decisions.len(),
                    scan.decisions.last().map(|d| d.junme).unwrap_or(0)
                ))
            });
        let m = dp.material.clone();
        println!(
            "junme {} actual {} tehai {}",
            m.junme,
            m.actual,
            m.tehai.join(" ")
        );
        let snap =
            akagi::tutorial::assemble::replay_snapshot_at(&events, seat, dp.event_index as usize)
                .expect("replay")
                .expect("snapshot");
        let info = akagi::analysis::snapshot_adapter::to_player_info(&snap, seat).expect("info");
        let result = akagi::analysis::analyze(&info);
        println!(
            "engine maintain top-6: (discard shanten waits_total score agari% riichi_pt dama_pt)"
        );
        for c in result
            .hand14
            .as_ref()
            .expect("hand14")
            .maintain
            .iter()
            .take(6)
        {
            println!(
                "  {:>4}  shanten={}  waits={:<3}  score={:.3}  agari={:.1}%  riichi={:.0}  dama={:.0}",
                c.discard,
                c.result.shanten,
                c.result.waits_total,
                c.result.mixed_waits_score,
                c.result.avg_agari_rate,
                c.result.riichi_point,
                c.result.dama_point,
            );
        }
        println!(
            "best_attack={:?} best_defence={:?} mixed_risk_len={}",
            result.best_attack_discard,
            result.best_defence_discard,
            result.mixed_risk.len()
        );
        return;
    }

    // --live-sim: replay the recorded game through the live-coach path
    // (single-pass tracker replay → build_live_turn → auto worker), the
    // same code the app runs when [tutorial.coach] auto_policy = "all".
    // `--fake` swaps in a scripted provider (fully offline);
    // `--no-consent` proves the outbound gate blocks; the session key is
    // derived from the game id, so a second run hits the disk cache and
    // must make zero provider calls.
    if live_sim {
        live_sim_run(
            &cfg, &events, seat, &record.id, fake, no_consent, limit, use_botref, effort, dump,
        )
        .await;
        return;
    }

    // Network path from here on — gate before the first request.
    if botref {
        let runtime = akagi::bot::PythonRuntime::locate(None)
            .ok()
            .unwrap_or_else(|| usage("no Python runtime found for the bot replay"));
        let doc = akagi::ipc::tutorial_botref::generate_bot_reference(
            &cfg,
            Some(runtime),
            &store,
            &history,
            &record.id,
            akagi::event_bus::notify_bus(),
        )
        .await
        .unwrap_or_else(|e| {
            eprintln!("bot reference failed: {e:#}");
            std::process::exit(1);
        });
        println!(
            "# bot {} — {} turns (created {})",
            doc.bot,
            doc.turns.len(),
            doc.created_at
        );
        for t in doc.turns.iter().take(12) {
            println!(
                "  junme {:>2} {} | {}",
                t.junme,
                t.action,
                t.picks
                    .iter()
                    .map(|p| format!("{} {:.0}%", p.tile, p.prob * 100.0))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if doc.turns.len() > 12 {
            println!("  … {} more", doc.turns.len() - 12);
        }
        return;
    }

    // --dump --fake is offline (scripted replies): it may run without
    // consent or a configured endpoint — no network is touched.
    if !cfg.tutorial.outbound_allowed() && !(dump && fake) {
        eprintln!(
            "outbound not allowed: enabled={} consent={} llm_configured={} \
             (set [tutorial] in config.toml and/or TUTORIAL_LLM_API_KEY)",
            cfg.tutorial.enabled,
            cfg.tutorial.upload_consent,
            cfg.tutorial.llm.is_configured(),
        );
        std::process::exit(2);
    }

    // --dump: render the exact prompt(s) uploaded and the LLM's raw
    // replies into one self-contained HTML for eyeballing input/output
    // quality. Re-run freely after changing config (model / coverage /
    // reasoning_effort) — the app never needs rebuilding for this.
    if dump {
        let botref = store.load_botref(&record.id).ok().flatten();
        dump_game(
            &cfg,
            &events,
            seat,
            &record.id,
            coverage,
            botref.as_ref(),
            fake,
        )
        .await;
        return;
    }

    let started = std::time::Instant::now();
    let outcome = generate_review(
        &cfg.tutorial,
        &store,
        &events,
        seat,
        &record.id,
        record.names.get(seat as usize).cloned(),
        coverage,
    )
    .await
    .unwrap_or_else(|e| {
        eprintln!("generate failed: {e:#}");
        std::process::exit(1);
    });
    println!(
        "# {} in {:?} — {} turns, model {:?}",
        if outcome.cached {
            "cache hit"
        } else {
            "generated"
        },
        started.elapsed(),
        outcome.doc.turns.len(),
        outcome.doc.model,
    );

    if json {
        println!("{}", serde_json::to_string_pretty(&outcome.doc).unwrap());
    } else {
        println!();
        println!(
            "【总评】{}",
            outcome.doc.overview.as_deref().unwrap_or("<none>")
        );
        println!();
        for t in &outcome.doc.turns {
            println!(
                "── 第 {} 巡 · 推荐 {} · 向听{} 和率{} 铳率{} ──",
                t.junme,
                t.recommended,
                t.numbers.shanten,
                t.numbers
                    .win_rate
                    .map(|v| format!("{:.0}%", v * 100.0))
                    .unwrap_or_else(|| "—".into()),
                t.numbers
                    .deal_in_rate
                    .map(|v| format!("{:.1}%", v * 100.0))
                    .unwrap_or_else(|| "—".into()),
            );
            if let Some(h) = &t.headline {
                println!("▍{h}");
            }
            println!("{}", t.explanation_zh);
            println!();
        }
    }
}

fn usage(msg: &str) -> ! {
    eprintln!("{msg}");
    eprintln!(
        "usage: tutorial-debug [--game <ulid|latest>] [--coverage key|all] [--refresh] [--json] [--inspect <junme>] [--botref] [--dump]\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20live-sim: [--live-sim [--fake] [--no-consent] [--use-botref] [--effort low|high|none] [--limit N] [--dump]]"
    );
    std::process::exit(2)
}

// ---------- --live-sim: auto live-turn pipeline smoke ----------

/// Scripted offline provider for `--fake`. The reply is shaped after
/// the prompt it was given (review array vs single turn) so the dump
/// viewer's JSON decomposition fills in like a real run: junme, the
/// first candidate's label and its numbers are echoed **verbatim from
/// the user prompt text**, which is exactly what the strict diagnostic
/// wants to see.
struct SimFakeProvider;

#[async_trait::async_trait]
impl akagi::tutorial::provider::TutorialProvider for SimFakeProvider {
    fn name(&self) -> &'static str {
        "sim-fake"
    }

    async fn complete(
        &self,
        req: akagi::tutorial::provider::ChatRequest,
    ) -> anyhow::Result<akagi::tutorial::provider::ChatResponse> {
        let user = req
            .messages
            .get(1)
            .map(|m| m.content.as_str())
            .unwrap_or_default();
        let content = fake_reply_for(user);
        Ok(akagi::tutorial::provider::ChatResponse {
            content,
            model: Some("sim-fake".into()),
            usage: None,
        })
    }
}

/// Parse the turn blocks out of a rendered user prompt and build a
/// shape-matching reply. Review prompts (「共 N 个决策点」) get a
/// ReviewDoc; single-turn prompts get a TurnDoc. Falls back to a canned
/// single turn when nothing parses.
fn fake_reply_for(user: &str) -> String {
    let mut turns: Vec<(u32, serde_json::Value)> = Vec::new();
    for block in user.split("=== 巡 ").skip(1) {
        let Some((junme, block)) = block.split_once(" ===") else {
            continue;
        };
        let Ok(junme) = junme.trim().parse::<u32>() else {
            continue;
        };
        let Some((label, numbers)) = parse_first_candidate(block) else {
            continue;
        };
        turns.push((
            junme,
            serde_json::json!({
                "junme": junme,
                "recommended": label,
                "numbers": numbers,
                "explanation_zh": "（离线脚本占位回显：结构与数字均取自 prompt 原文。）",
                "headline": "离线占位",
            }),
        ));
    }
    let values: Vec<serde_json::Value> = turns.into_iter().map(|(_, v)| v).collect();
    if user.contains("个决策点") && !values.is_empty() {
        serde_json::json!({
            "overview": "（离线脚本占位回显：结构与数字均取自 prompt 原文。）",
            "turns": values,
        })
        .to_string()
    } else if let Some(first) = values.into_iter().next() {
        first.to_string()
    } else {
        serde_json::json!({
            "junme": 0,
            "recommended": "dahai:E",
            "numbers": {},
            "explanation_zh": "模拟讲解：先切孤张字牌，保留两面搭子推进一向听。",
            "headline": "模拟巡讲解",
        })
        .to_string()
    }
}

/// `(label, numbers)` of the first candidate line of one turn block —
/// echoed verbatim so strict cross-checks pass offline too.
fn parse_first_candidate(block: &str) -> Option<(String, serde_json::Value)> {
    let cand = block.find("候选:")?;
    let line_start = cand + block[cand..].find("  1. ")? + 5;
    let line = &block[line_start..];
    let line = &line[..line.find('\n')?];
    let label = line[..line.find(" (p=")?].to_string();
    let tail = &line[line.find(") ")? + 2..];
    let mut it = tail.split(' ');
    let shanten: i32 = it.next()?.trim_start_matches("向听").parse().ok()?;
    let num = |s: Option<&str>| -> Option<serde_json::Value> {
        let s = s?.trim_start_matches(|c: char| !c.is_ascii_digit() && c != '-' && c != 'n');
        if s == "null" || s.is_empty() {
            return None;
        }
        s.parse::<f64>().ok().map(serde_json::Value::from)
    };
    let win = num(it.next());
    let risk = num(it.next());
    let score = it
        .next()
        .and_then(|s| s.trim_start_matches("打点").parse::<i64>().ok());
    Some((
        label,
        serde_json::json!({
            "shanten": shanten,
            "win_rate": win,
            "deal_in_rate": risk,
            "avg_score": score,
        }),
    ))
}

/// One captured live-sim request: exactly what left the machine and what
/// came back, with the wall-clock latency — the input side of the dump
/// viewer when `--live-sim --dump` is used.
#[derive(Clone)]
struct SimRecord {
    system: String,
    user: String,
    reply: String,
    model: Option<String>,
    usage: Option<akagi::tutorial::provider::ChatUsage>,
    elapsed_ms: u128,
}

/// Wraps a provider and records every outbound attempt (count + the full
/// request/response pair), so the sim can prove "second run = 0 network
/// calls" and render what the app actually uploads per turn.
struct CountingProvider {
    inner: std::sync::Arc<dyn akagi::tutorial::provider::TutorialProvider>,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    records: std::sync::Arc<std::sync::Mutex<Vec<SimRecord>>>,
}

#[async_trait::async_trait]
impl akagi::tutorial::provider::TutorialProvider for CountingProvider {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    async fn complete(
        &self,
        req: akagi::tutorial::provider::ChatRequest,
    ) -> anyhow::Result<akagi::tutorial::provider::ChatResponse> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let system = req
            .messages
            .first()
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let user = req
            .messages
            .get(1)
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let t0 = std::time::Instant::now();
        let resp = self.inner.complete(req).await?;
        self.records.lock().unwrap().push(SimRecord {
            system,
            user,
            reply: resp.content.clone(),
            model: resp.model.clone(),
            usage: resp.usage.clone(),
            elapsed_ms: t0.elapsed().as_millis(),
        });
        Ok(resp)
    }
}

#[allow(clippy::too_many_arguments)] // private dev-tool entry point
async fn live_sim_run(
    cfg: &akagi::config::AppConfig,
    events: &[akagi::schema::MjaiEvent],
    seat: u8,
    game_id: &str,
    fake: bool,
    no_consent: bool,
    limit: Option<usize>,
    use_botref: bool,
    effort: Option<String>,
    dump: bool,
) {
    use akagi::game_state::tracker::GameTracker;
    use akagi::schema::MjaiEvent;
    use akagi::tutorial::live::{auto_explain_worker, AutoJob};
    use akagi::tutorial::provider::TutorialProvider;

    // The sim always runs the "policy on" scenario (that is the feature
    // under test); consent/endpoint are togglable to prove the gates.
    let mut sim_cfg = cfg.clone();
    sim_cfg.tutorial.enabled = true;
    sim_cfg.tutorial.upload_consent = !no_consent;
    sim_cfg.tutorial.coach.enabled = true;
    sim_cfg.tutorial.coach.auto_policy = akagi::config::AutoPolicy::All;
    if let Some(e) = effort {
        sim_cfg.tutorial.llm.reasoning_effort = if e == "none" { None } else { Some(e) };
    }
    if fake {
        // The fake factory never contacts anything, but the gates still
        // require a "configured" endpoint — stamp sim credentials so the
        // --fake run exercises the full queue→worker→save→event path
        // offline regardless of what config.toml holds.
        sim_cfg.tutorial.llm.base_url = "http://127.0.0.1:9/v1".into();
        sim_cfg.tutorial.llm.api_key = "sim".into();
        sim_cfg.tutorial.llm.model = "sim-fake".into();
    }

    // --use-botref: replay each decision with the recorded Mortal answers
    // (real candidate probabilities), so the prompts match what a live
    // game uploads. Without it, the synthetic answer carries no show card
    // and build_live_turn falls back to engine-top candidates.
    let botref = if use_botref {
        match akagi::tutorial::TutorialStore::new(resolve_dir(std::path::Path::new("./tutorial")))
            .expect("open tutorial store")
            .load_botref(game_id)
        {
            Ok(Some(d)) => {
                println!(
                    "# botref: {} turns from bot {} (recorded {})",
                    d.turns.len(),
                    d.bot,
                    d.created_at
                );
                Some(d)
            }
            Ok(None) => {
                eprintln!("no bot reference for {game_id} — run with --botref first");
                std::process::exit(2);
            }
            Err(e) => {
                eprintln!("botref load failed: {e:#}");
                std::process::exit(2);
            }
        }
    } else {
        None
    };

    // Single-pass replay: feed the tracker (with StartGame seat
    // injection, same as scan_decisions) and build the LIVE material at
    // every own-draw decision point via the production entry point.
    let session_key = format!("sim-{game_id}");
    let mut tracker = GameTracker::new();
    let mut our_draws = 0u32;
    let mut jobs: Vec<AutoJob> = Vec::new();
    // junme → material, for pairing the dump viewer's decomposition table.
    let mut materials: std::collections::HashMap<u32, akagi::tutorial::material::TurnMaterial> =
        std::collections::HashMap::new();
    for ev in events {
        let ev = match ev {
            MjaiEvent::StartGame { id, .. } if id.is_none() => {
                let mut patched = ev.clone();
                if let MjaiEvent::StartGame { id, .. } = &mut patched {
                    *id = Some(seat);
                }
                patched
            }
            other => other.clone(),
        };
        if let MjaiEvent::Tsumo { actor, .. } = &ev {
            if *actor == seat {
                our_draws += 1;
            }
        }
        if let Err(e) = tracker.handle(&ev) {
            eprintln!("# tracker stopped at this event: {e:#} — stopping replay");
            break;
        }
        let is_our_draw = matches!(&ev, MjaiEvent::Tsumo { actor, .. } if *actor == seat);
        if !is_our_draw {
            continue;
        }
        let Some(snap) = tracker.snapshot() else {
            continue;
        };
        let drawn = snap.players[seat as usize]
            .drawn_tile
            .clone()
            .or_else(|| snap.players[seat as usize].tehai.last().cloned())
            .unwrap_or_default();
        // Synthetic bot answer: with --use-botref, the recorded Mortal
        // action + ranked show card (real probabilities); otherwise a
        // tsumogiri with no show card (engine-top fallback).
        let bot_turn = botref
            .as_ref()
            .and_then(|b| b.turns.iter().find(|t| t.junme == our_draws));
        let (action, meta) = match bot_turn {
            Some(t) => {
                let items: Vec<serde_json::Value> = t
                    .picks
                    .iter()
                    .map(
                        |p| serde_json::json!({"label": p.label, "pais": [p.tile], "prob": p.prob}),
                    )
                    .collect();
                let meta = Some(serde_json::json!({"show": {"title": "sim", "items": items}}));
                let action = if let Some(tile) = t.action.strip_prefix("dahai:") {
                    MjaiEvent::Dahai {
                        actor: seat,
                        pai: tile.into(),
                        tsumogiri: false,
                    }
                } else if t.action == "reach" {
                    MjaiEvent::Reach {
                        actor: seat,
                        pai: None,
                    }
                } else {
                    MjaiEvent::Dahai {
                        actor: seat,
                        pai: drawn.clone(),
                        tsumogiri: true,
                    }
                };
                (action, meta)
            }
            None => (
                MjaiEvent::Dahai {
                    actor: seat,
                    pai: drawn,
                    tsumogiri: true,
                },
                None,
            ),
        };
        let resp = akagi::bot::BotResponse { action, meta };
        if let Some(material) = akagi::tutorial::live::build_live_turn(&snap, &resp, our_draws) {
            materials.insert(material.junme, material.clone());
            jobs.push(AutoJob {
                session_key: session_key.clone(),
                material,
            });
        }
    }
    let total_points = jobs.len();
    if let Some(n) = limit {
        jobs.truncate(n);
    }
    println!(
        "# live-sim {} — {} decision points, {} queued, session {}, provider {}",
        game_id,
        total_points,
        jobs.len(),
        session_key,
        if fake { "fake" } else { "real" },
    );
    println!(
        "# gates: policy=all consent={} endpoint_configured={}",
        sim_cfg.tutorial.upload_consent,
        sim_cfg.tutorial.llm.is_configured(),
    );

    let ctx = std::sync::Arc::new(
        akagi::tutorial::TutorialContext::new(
            resolve_dir(std::path::Path::new("./tutorial")),
            akagi::tutorial::LiveCoachHandle::default(),
        )
        .expect("tutorial context"),
    );
    let (bus, _keeper) = tokio::sync::broadcast::channel(1024);
    let mut sub = bus.subscribe();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let records: std::sync::Arc<std::sync::Mutex<Vec<SimRecord>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let factory_calls = calls.clone();
    let factory_records = records.clone();
    let factory = move |llm: &akagi::config::LlmConfig| -> anyhow::Result<
        std::sync::Arc<dyn akagi::tutorial::provider::TutorialProvider>,
    > {
        let inner: std::sync::Arc<dyn TutorialProvider> = if fake {
            std::sync::Arc::new(SimFakeProvider)
        } else {
            std::sync::Arc::new(akagi::tutorial::OpenAiCompatProvider::new(llm)?)
        };
        Ok(std::sync::Arc::new(CountingProvider {
            inner,
            calls: factory_calls.clone(),
            records: factory_records.clone(),
        }))
    };

    let started = std::time::Instant::now();
    let queued = jobs.len();
    let sim_llm = sim_cfg.tutorial.llm.clone();
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let worker = tokio::spawn(auto_explain_worker(
        rx,
        ctx.clone(),
        std::sync::Arc::new(tokio::sync::RwLock::new(sim_cfg)),
        bus.clone(),
        factory,
    ));
    for j in jobs {
        let _ = tx.send(j).await;
    }
    drop(tx);
    worker.await.expect("worker join");

    let mut events_out = Vec::new();
    while let Ok(ev) = sub.try_recv() {
        events_out.push(ev);
    }
    let provider_calls = calls.load(std::sync::atomic::Ordering::SeqCst);
    let fresh = provider_calls.min(events_out.len());
    let cache_hits = events_out.len() - fresh;
    println!("# --- summary ---");
    println!(
        "# queued {} → events {}, provider calls {}, approx cache hits {}, total {:?}",
        queued,
        events_out.len(),
        provider_calls,
        cache_hits,
        started.elapsed(),
    );
    if !records.lock().unwrap().is_empty() {
        let list: Vec<u128> = records
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.elapsed_ms)
            .collect();
        let min = list.iter().min().unwrap();
        let max = list.iter().max().unwrap();
        let avg = list.iter().sum::<u128>() / list.len() as u128;
        println!(
            "# per-call latency: min {:.1}s · avg {:.1}s · max {:.1}s (n={})",
            *min as f64 / 1000.0,
            avg as f64 / 1000.0,
            *max as f64 / 1000.0,
            list.len()
        );
    }
    for ev in events_out.iter().take(8) {
        println!(
            "  junme {:>2} [{}] {}",
            ev.junme,
            ev.model.as_deref().unwrap_or("?"),
            ev.doc
                .headline
                .as_deref()
                .unwrap_or(&ev.doc.explanation_zh)
                .chars()
                .take(40)
                .collect::<String>()
        );
    }
    if events_out.len() > 8 {
        println!("  … {} more", events_out.len() - 8);
    }

    // --live-sim --dump: render what each turn actually uploaded / got
    // back into the shared viewer HTML, latency badge per turn.
    if dump {
        let records = records.lock().unwrap().clone();
        if records.is_empty() {
            println!("# nothing to dump (all turns came from cache)");
        } else {
            dump_live_sim(game_id, &sim_llm, records, &materials);
        }
    }
}

/// Render live-sim records through the shared viewer: each record is one
/// "chunk" (one turn per request), junme parsed back out of the user
/// message, decomposition paired with the material by junme.
fn dump_live_sim(
    game_id: &str,
    llm: &akagi::config::LlmConfig,
    records: Vec<SimRecord>,
    materials: &std::collections::HashMap<u32, akagi::tutorial::material::TurnMaterial>,
) {
    let chunks: Vec<DumpChunk> = records
        .iter()
        .map(|r| {
            // The marker is UTF-8 multi-byte ("巡" = 3 bytes) — slice on
            // the marker's own len, never a hand-counted offset.
            let marker = "=== 巡 ";
            let junme = r
                .user
                .find(marker)
                .and_then(|p| {
                    let rest = &r.user[p + marker.len()..];
                    let end = rest.find(" ===")?;
                    rest[..end].parse::<u32>().ok()
                })
                .unwrap_or(0);
            let material = materials.get(&junme);
            let strict: Vec<String> = match material {
                Some(m) => match akagi::tutorial::download::parse_turn_doc(&r.reply, m) {
                    Ok(_) => Vec::new(),
                    Err(e) => vec![format!("{e:#}")],
                },
                None => vec!["material missing for this turn".into()],
            };
            // Trust-parse against a stub carrying the real junme: the
            // doc is kept verbatim either way, junme is stamped locally.
            let stub = akagi::tutorial::material::TurnMaterial {
                junme,
                ..Default::default()
            };
            let (turns, parse_error) =
                match akagi::tutorial::download::parse_turn_doc_trust(&r.reply, &stub) {
                    Ok(doc) => {
                        let material_numbers = material.and_then(|m| {
                            m.candidates
                                .iter()
                                .find(|c| c.label == doc.recommended)
                                .map(|c| c.numbers.clone())
                        });
                        (
                            vec![DumpTurn {
                                junme,
                                recommended: doc.recommended.clone(),
                                numbers: doc.numbers.clone(),
                                headline: doc.headline.clone(),
                                explanation: doc.explanation_zh.clone(),
                                material_numbers,
                            }],
                            None,
                        )
                    }
                    Err(e) => (Vec::new(), Some(format!("{e:#}"))),
                };
            DumpChunk {
                system: r.system.clone(),
                user: r.user.clone(),
                raw_reply: r.reply.clone(),
                model: r.model.clone(),
                usage: r.usage.clone(),
                elapsed_ms: r.elapsed_ms,
                strict_kept: usize::from(strict.is_empty()),
                strict_rejected: strict,
                overview: None,
                turns,
                parse_error,
            }
        })
        .collect();

    let lat: Vec<u128> = records.iter().map(|r| r.elapsed_ms).collect();
    let min = *lat.iter().min().unwrap_or(&0);
    let max = *lat.iter().max().unwrap_or(&0);
    let avg = lat.iter().sum::<u128>() / lat.len().max(1) as u128;
    let mode = format!(
        "live-sim 抢先单巡 · model {} · effort {} · 单巡时延 min {:.1}s / avg {:.1}s / max {:.1}s (n={})",
        llm.model,
        llm.reasoning_effort.as_deref().unwrap_or("未设"),
        min as f64 / 1000.0,
        avg as f64 / 1000.0,
        max as f64 / 1000.0,
        lat.len(),
    );

    let mut dir = resolve_dir(std::path::Path::new("./tutorial"))
        .parent()
        .unwrap()
        .to_path_buf();
    dir.push("tutorial-dump");
    dir.push(format!(
        "live-{}-{}",
        game_id,
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    std::fs::create_dir_all(&dir).expect("create dump dir");
    let html = render_dump_html(game_id, &mode, &chunks);
    let view = dir.join("view.html");
    std::fs::write(&view, html).expect("write view.html");
    for (i, c) in chunks.iter().enumerate() {
        std::fs::write(
            dir.join(format!("prompt-{}.txt", i + 1)),
            format!("{}\n\n{}", c.system, c.user),
        )
        .ok();
        std::fs::write(dir.join(format!("reply-{}.txt", i + 1)), &c.raw_reply).ok();
    }
    println!("# live-sim dump written: {}", view.display());
    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .args(["/C", "start", "", &view.display().to_string()])
        .spawn()
        .ok();
}

// ---------- --dump: prompt / reply viewer (self-contained HTML) ----------

/// One turn of the reply, decomposed from JSON and paired with the
/// material's numbers for the same candidate (viewer-side comparison).
struct DumpTurn {
    junme: u32,
    recommended: String,
    numbers: akagi::tutorial::material::EngineNumbers,
    headline: Option<String>,
    explanation: String,
    /// The material candidate matching `recommended`, when one exists.
    material_numbers: Option<akagi::tutorial::material::EngineNumbers>,
}

impl DumpTurn {
    fn matches_material(&self) -> Option<bool> {
        self.material_numbers.as_ref().map(|m| *m == self.numbers)
    }
}

struct DumpChunk {
    system: String,
    user: String,
    raw_reply: String,
    model: Option<String>,
    usage: Option<akagi::tutorial::provider::ChatUsage>,
    elapsed_ms: u128,
    /// Strict cross-check run purely as a DIAGNOSTIC for the viewer —
    /// the pipeline itself is trust-passthrough (D-0013).
    strict_rejected: Vec<String>,
    strict_kept: usize,
    /// JSON-decomposed turns (empty when the reply wasn't keepable JSON).
    overview: Option<String>,
    turns: Vec<DumpTurn>,
    parse_error: Option<String>,
}

async fn dump_game(
    cfg: &akagi::config::AppConfig,
    events: &[akagi::schema::MjaiEvent],
    seat: u8,
    game_id: &str,
    coverage: ReviewCoverage,
    botref: Option<&akagi::tutorial::material::BotRefDoc>,
    fake: bool,
) {
    use akagi::tutorial::provider::{ChatRequest, OpenAiCompatProvider, TutorialProvider};

    let material =
        akagi::tutorial::build_review_material(events, seat, game_id, None, coverage, botref)
            .expect("assemble material");
    if material.turns.is_empty() {
        eprintln!("no reviewable decisions for this game/coverage");
        std::process::exit(2);
    }
    let redacted = akagi::tutorial::upload::redact(&material);
    // --fake swaps in the scripted offline provider: the viewer then
    // shows exactly what WOULD be uploaded, without spending a call.
    let provider: std::sync::Arc<dyn TutorialProvider> = if fake {
        std::sync::Arc::new(SimFakeProvider)
    } else {
        std::sync::Arc::new(OpenAiCompatProvider::new(&cfg.tutorial.llm).expect("provider"))
    };

    let mut chunks: Vec<DumpChunk> = Vec::new();
    for chunk in redacted.turns.chunks(6) {
        let sub = akagi::tutorial::material::ReviewMaterial {
            game_id: redacted.game_id.clone(),
            seat: redacted.seat,
            num_players: redacted.num_players,
            player_name: None,
            turns: chunk.to_vec(),
        };
        let msgs = akagi::tutorial::upload::build_messages(&sub);
        let t0 = std::time::Instant::now();
        let resp = provider
            .complete(ChatRequest {
                messages: msgs.clone(),
                temperature: Some(0.2),
                max_tokens: Some(16000),
                reasoning_effort: cfg.tutorial.llm.reasoning_effort.clone(),
            })
            .await
            .unwrap_or_else(|e| {
                eprintln!("LLM request failed: {e:#}");
                std::process::exit(1);
            });
        let elapsed_ms = t0.elapsed().as_millis();
        let (strict_kept, strict_rejected) =
            match akagi::tutorial::download::parse_review_doc_lenient(&resp.content, &sub) {
                Ok((doc, rej)) => (doc.turns.len(), rej),
                Err(e) => (0, vec![format!("parse failed outright: {e:#}")]),
            };
        // JSON decomposition for the viewer: keep the reply verbatim,
        // pair each turn's numbers with the material candidate's.
        let (overview, turns, parse_error) =
            match akagi::tutorial::download::parse_review_doc_trust(&resp.content, &sub) {
                Ok(doc) => {
                    let turns = doc
                        .turns
                        .iter()
                        .map(|t| {
                            let material_numbers =
                                sub.turns.iter().find(|m| m.junme == t.junme).and_then(|m| {
                                    m.candidates
                                        .iter()
                                        .find(|c| c.label == t.recommended)
                                        .map(|c| c.numbers.clone())
                                });
                            DumpTurn {
                                junme: t.junme,
                                recommended: t.recommended.clone(),
                                numbers: t.numbers.clone(),
                                headline: t.headline.clone(),
                                explanation: t.explanation_zh.clone(),
                                material_numbers,
                            }
                        })
                        .collect();
                    (doc.overview.clone(), turns, None)
                }
                Err(e) => (None, Vec::new(), Some(format!("{e:#}"))),
            };
        chunks.push(DumpChunk {
            system: msgs[0].content.clone(),
            user: msgs[1].content.clone(),
            raw_reply: resp.content.clone(),
            model: resp.model.clone(),
            usage: resp.usage.clone(),
            elapsed_ms,
            strict_rejected,
            strict_kept,
            overview,
            turns,
            parse_error,
        });
    }

    // <config_root>/tutorial-dump/<game>-<hhmmss>/view.html (+ raw txts)
    let mut dir = resolve_dir(std::path::Path::new("./tutorial"))
        .parent()
        .unwrap()
        .to_path_buf();
    dir.push("tutorial-dump");
    dir.push(format!(
        "{}-{}",
        game_id,
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    std::fs::create_dir_all(&dir).expect("create dump dir");
    let html = render_dump_html(
        game_id,
        &format!(
            "复盘 · model <b>{}</b> · 覆盖 <b>{}</b>",
            esc(&cfg.tutorial.llm.model),
            match coverage {
                ReviewCoverage::Key => "关键巡",
                ReviewCoverage::All => "全部巡",
            }
        ),
        &chunks,
    );
    let view = dir.join("view.html");
    std::fs::write(&view, html).expect("write view.html");
    for (i, c) in chunks.iter().enumerate() {
        std::fs::write(
            dir.join(format!("prompt-{}.txt", i + 1)),
            format!("{}\n\n{}", c.system, c.user),
        )
        .ok();
        std::fs::write(dir.join(format!("reply-{}.txt", i + 1)), &c.raw_reply).ok();
    }
    println!("# dump written: {}", view.display());
    // Open in the default browser (Windows); failure is non-fatal.
    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .args(["/C", "start", "", &view.display().to_string()])
        .spawn()
        .ok();
}

fn fmt_numbers(n: &akagi::tutorial::material::EngineNumbers) -> String {
    let pct = |v: Option<f64>| match v {
        Some(v) => format!("{:.1}%", v * 100.0),
        None => "—".into(),
    };
    format!(
        "向听{} · 和率{} · 铳率{} · 打点{}",
        n.shanten,
        pct(n.win_rate),
        pct(n.deal_in_rate),
        n.avg_score
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".into())
    )
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn render_dump_html(game_id: &str, mode: &str, chunks: &[DumpChunk]) -> String {
    let mut h = String::new();
    h.push_str("<!doctype html><html lang=\"zh\"><head><meta charset=\"utf-8\">");
    h.push_str("<title>教程 LLM 输入/输出查看器</title><style>");
    h.push_str("body{font-family:system-ui,'Microsoft YaHei',sans-serif;margin:0;background:#111;color:#ddd;}");
    h.push_str("h1{font-size:18px;padding:12px 16px;background:#1b1b1b;margin:0;}");
    h.push_str(".meta{padding:8px 16px;color:#888;font-size:13px;border-bottom:1px solid #333;}");
    h.push_str("section{padding:12px 16px;border-bottom:1px solid #333;}");
    h.push_str("h2{font-size:15px;color:#7cc4ff;margin:8px 0;}");
    h.push_str("pre{background:#181818;border:1px solid #333;border-radius:6px;padding:10px;white-space:pre-wrap;word-break:break-all;font-size:12px;line-height:1.55;max-height:420px;overflow:auto;}");
    h.push_str(".cols{display:grid;grid-template-columns:1fr 1fr;gap:10px;}");
    h.push_str("@media(max-width:900px){.cols{grid-template-columns:1fr;}}");
    h.push_str("table{border-collapse:collapse;width:100%;font-size:13px;}");
    h.push_str("td,th{border:1px solid #333;padding:4px 8px;text-align:left;vertical-align:top;}");
    h.push_str(".ok{color:#6c6;}.bad{color:#e66;}.warn{color:#eb6;}");
    h.push_str(".tag{display:inline-block;background:#333;border-radius:4px;padding:1px 6px;margin-right:6px;font-size:11px;color:#bbb;}");
    h.push_str("</style></head><body>");
    h.push_str(&format!(
        "<h1>教程 LLM 输入/输出查看器 · {}</h1>",
        esc(game_id)
    ));
    h.push_str(&format!(
        "<div class=\"meta\">{} · 决策 {} 巡 / {} 批 · 生成于 {} · 管线为信任直通(D-0013)，下方核对仅为诊断</div>",
        mode,
        chunks.iter().map(|c| c.user.matches("=== 巡").count()).sum::<usize>(),
        chunks.len(),
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
    ));

    for (i, c) in chunks.iter().enumerate() {
        let usage = match &c.usage {
            Some(u) => format!(
                "prompt {} tok · completion {} tok",
                u.prompt_tokens.unwrap_or(0),
                u.completion_tokens.unwrap_or(0)
            ),
            None => "usage 未上报".into(),
        };
        h.push_str(&format!(
            "<section><h2>第 {} 批 <span class=\"tag\">{}</span><span class=\"tag\">{} ms</span><span class=\"tag\">{}</span><span class=\"tag\">严格核对: <b class=\"{}\">{} 通过 / {} 拒绝</b></span></h2>",
            i + 1,
            esc(c.model.as_deref().unwrap_or("?")),
            c.elapsed_ms,
            usage,
            if c.strict_rejected.is_empty() { "ok" } else { "warn" },
            c.strict_kept,
            c.strict_rejected.len(),
        ));
        h.push_str("<div class=\"cols\"><div><h2>上传的 Prompt（system + user 原文）</h2><pre>");
        h.push_str(&esc(&format!(
            "【system】\n{}\n\n【user】\n{}",
            c.system, c.user
        )));
        h.push_str("</pre></div><div><h2>LLM 原始输出（未做任何删改）</h2><pre>");
        h.push_str(&esc(&c.raw_reply));
        h.push_str("</pre></div></div>");
        // JSON 分解：把回显解析成 总评 + 逐巡表格，并与原料数字并排对照。
        h.push_str("<h2>JSON 分解（回显解析 + 与原料对照）</h2>");
        if let Some(err) = &c.parse_error {
            h.push_str(&format!(
                "<p class=\"bad\">回显无法解析为 JSON：<code>{}</code></p>",
                esc(err)
            ));
        } else {
            if let Some(ov) = &c.overview {
                h.push_str(&format!("<p><b>总评：</b>{}</p>", esc(ov)));
            }
            h.push_str("<table><tr><th>巡</th><th>recommended</th><th>回显数字</th><th>原料数字</th><th>核对</th><th>headline</th><th>讲解</th></tr>");
            for t in &c.turns {
                let verdict = match t.matches_material() {
                    Some(true) => "<span class=\"ok\">一致</span>".to_string(),
                    Some(false) => "<span class=\"bad\">不一致</span>".to_string(),
                    None => "<span class=\"warn\">候选外</span>".to_string(),
                };
                let cell_class = match t.matches_material() {
                    Some(true) => "",
                    _ => " class=\"bad\"",
                };
                h.push_str(&format!(
                    "<tr><td>{}</td><td><code>{}</code></td>{}<td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    t.junme,
                    esc(&t.recommended),
                    format_args!("<td{}>{}</td>", cell_class, esc(&fmt_numbers(&t.numbers))),
                    esc(&t
                        .material_numbers
                        .as_ref()
                        .map(fmt_numbers)
                        .unwrap_or_else(|| "—".into())),
                    verdict,
                    esc(t.headline.as_deref().unwrap_or("")),
                    esc(&t.explanation),
                ));
            }
            h.push_str("</table>");
        }
        if !c.strict_rejected.is_empty() {
            h.push_str("<h2>诊断 · 若按严格核对会拒绝的巡</h2><ul>");
            for r in &c.strict_rejected {
                h.push_str(&format!("<li class=\"bad\">{}</li>", esc(r)));
            }
            h.push_str("</ul>");
        }
        h.push_str("</section>");
    }
    h.push_str("</body></html>");
    h
}
