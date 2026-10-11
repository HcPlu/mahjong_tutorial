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
        live_sim_run(&cfg, &events, seat, &record.id, fake, no_consent, limit).await;
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

    if !cfg.tutorial.outbound_allowed() {
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
        dump_game(&cfg, &events, seat, &record.id, coverage).await;
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
         \x20\x20\x20\x20\x20\x20\x20\x20\x20live-sim: [--live-sim [--fake] [--no-consent] [--limit N]]"
    );
    std::process::exit(2)
}

// ---------- --live-sim: auto live-turn pipeline smoke ----------

/// Scripted offline provider for `--fake`.
struct SimFakeProvider;

#[async_trait::async_trait]
impl akagi::tutorial::provider::TutorialProvider for SimFakeProvider {
    fn name(&self) -> &'static str {
        "sim-fake"
    }

    async fn complete(
        &self,
        _req: akagi::tutorial::provider::ChatRequest,
    ) -> anyhow::Result<akagi::tutorial::provider::ChatResponse> {
        Ok(akagi::tutorial::provider::ChatResponse {
            content: serde_json::json!({
                "junme": 0,
                "recommended": "dahai:E",
                "numbers": {},
                "explanation_zh": "模拟讲解：先切孤张字牌，保留两面搭子推进一向听。",
                "headline": "模拟巡讲解",
            })
            .to_string(),
            model: Some("sim-fake".into()),
            usage: None,
        })
    }
}

/// Wraps a provider and records every outbound attempt (count + elapsed),
/// so the sim can prove "second run = 0 network calls".
struct CountingProvider {
    inner: std::sync::Arc<dyn akagi::tutorial::provider::TutorialProvider>,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    elapsed_ms: std::sync::Arc<std::sync::Mutex<Vec<u128>>>,
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
        let t0 = std::time::Instant::now();
        let resp = self.inner.complete(req).await?;
        self.elapsed_ms
            .lock()
            .unwrap()
            .push(t0.elapsed().as_millis());
        Ok(resp)
    }
}

async fn live_sim_run(
    cfg: &akagi::config::AppConfig,
    events: &[akagi::schema::MjaiEvent],
    seat: u8,
    game_id: &str,
    fake: bool,
    no_consent: bool,
    limit: Option<usize>,
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
    sim_cfg.tutorial.coach.auto_policy = akagi::config::AutoPolicy::All;
    if fake {
        // The fake factory never contacts anything, but the gates still
        // require a "configured" endpoint — stamp sim credentials so the
        // --fake run exercises the full queue→worker→save→event path
        // offline regardless of what config.toml holds.
        sim_cfg.tutorial.llm.base_url = "http://127.0.0.1:9/v1".into();
        sim_cfg.tutorial.llm.api_key = "sim".into();
        sim_cfg.tutorial.llm.model = "sim-fake".into();
    }

    // Single-pass replay: feed the tracker (with StartGame seat
    // injection, same as scan_decisions) and build the LIVE material at
    // every own-draw decision point via the production entry point.
    let session_key = format!("sim-{game_id}");
    let mut tracker = GameTracker::new();
    let mut our_draws = 0u32;
    let mut jobs: Vec<AutoJob> = Vec::new();
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
        // Synthetic bot answer: tsumogiri of the drawn tile, no show card
        // → build_live_turn falls back to engine-top candidates.
        let drawn = snap.players[seat as usize]
            .drawn_tile
            .clone()
            .or_else(|| snap.players[seat as usize].tehai.last().cloned())
            .unwrap_or_default();
        let resp = akagi::bot::BotResponse {
            action: MjaiEvent::Dahai {
                actor: seat,
                pai: drawn,
                tsumogiri: true,
            },
            meta: None,
        };
        if let Some(material) = akagi::tutorial::live::build_live_turn(&snap, &resp, our_draws) {
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
    let elapsed_ms = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let factory_calls = calls.clone();
    let factory_elapsed = elapsed_ms.clone();
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
            elapsed_ms: factory_elapsed.clone(),
        }))
    };

    let started = std::time::Instant::now();
    let queued = jobs.len();
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
    if !elapsed_ms.lock().unwrap().is_empty() {
        let list = elapsed_ms.lock().unwrap().clone();
        let min = list.iter().min().unwrap();
        let max = list.iter().max().unwrap();
        let avg = list.iter().sum::<u128>() / list.len() as u128;
        println!(
            "# per-call latency: min {min} ms · avg {avg} ms · max {max} ms (n={})",
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
) {
    use akagi::tutorial::provider::{ChatRequest, OpenAiCompatProvider, TutorialProvider};

    let material = akagi::tutorial::build_review_material(events, seat, game_id, None, coverage)
        .expect("assemble material");
    if material.turns.is_empty() {
        eprintln!("no reviewable decisions for this game/coverage");
        std::process::exit(2);
    }
    let redacted = akagi::tutorial::upload::redact(&material);
    let provider = OpenAiCompatProvider::new(&cfg.tutorial.llm).expect("provider");

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
    let html = render_dump_html(game_id, &cfg.tutorial.llm.model, coverage, &chunks);
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

fn render_dump_html(
    game_id: &str,
    model: &str,
    coverage: ReviewCoverage,
    chunks: &[DumpChunk],
) -> String {
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
    let cov = match coverage {
        ReviewCoverage::Key => "关键巡",
        ReviewCoverage::All => "全部巡",
    };
    h.push_str(&format!(
        "<div class=\"meta\">model <b>{}</b> · 覆盖 <b>{}</b> · 决策 {} 巡 / {} 批 · 生成于 {} · 管线为信任直通(D-0013)，下方核对仅为诊断</div>",
        esc(model),
        cov,
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
