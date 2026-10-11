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
        "usage: tutorial-debug [--game <ulid|latest>] [--coverage key|all] [--refresh] [--json] [--inspect <junme>] [--botref]"
    );
    std::process::exit(2)
}
