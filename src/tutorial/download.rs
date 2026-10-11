//! Download zone — the **only** path cloud data takes into this app
//! (D-0007).
//!
//! Two regimes live here:
//!
//! - **Trust-passthrough (current stage, D-0013)**:
//!   [`parse_review_doc_trust`] / [`parse_turn_doc_trust`] keep the LLM's
//!   output verbatim — JSON extraction and identity stamping only, no
//!   turn dropping, no number cross-checking. The owner scoped the
//!   strict guard out for this stage of development.
//! - **Strict anti-hallucination (D-0005, parked for later)**:
//!   [`parse_review_doc`] / [`parse_review_doc_lenient`] /
//!   [`parse_turn_doc`] cross-check every echoed number against the
//!   material and reject what doesn't match. Kept compiled and tested —
//!   flip the pipeline back when the stage ends.

use anyhow::{bail, Context, Result};

use crate::tutorial::material::{EngineNumbers, ReviewDoc, ReviewMaterial};

/// Parse and validate an LLM review reply against its material.
///
/// Checks, in order:
/// 1. a JSON object is present (leading prose is tolerated — the object
///    spans the first `{` to the last `}`),
/// 2. the reply's turn count matches the material's, junme by junme,
/// 3. each turn's `recommended` label names a candidate the material
///    actually offered,
/// 4. each turn's `numbers` **exactly equal** that candidate's engine
///    numbers (floats compare bit-exact: `upload` renders them
///    round-trip, so a faithful echo parses to the same value),
/// 5. each turn carries non-empty Chinese explanation text.
///
/// `game_id` / `seat` are taken from the material, never from the reply.
/// `created_at` / `model` are left for the caller to stamp.
///
/// A turn with no engine candidates in the material cannot be reviewed
/// under these rules and fails loudly here — the pipeline should not
/// put candidate-less turns into the material in the first place.
pub fn parse_review_doc(content: &str, material: &ReviewMaterial) -> Result<ReviewDoc> {
    let json_span = extract_json_span(content).context("no JSON object found in LLM reply")?;
    let mut doc: ReviewDoc = serde_json::from_str(json_span)
        .with_context(|| "LLM reply is not valid JSON for a review document")?;

    if doc.turns.len() != material.turns.len() {
        bail!(
            "turn count mismatch: reply has {}, material has {} — \
             ask again or degrade to numbers-only",
            doc.turns.len(),
            material.turns.len()
        );
    }

    for (i, (turn_doc, turn_mat)) in doc.turns.iter().zip(&material.turns).enumerate() {
        let at = format!("turn #{i} (junme {})", turn_mat.junme);
        if turn_doc.junme != turn_mat.junme {
            bail!(
                "{at}: junme {} does not match material {}",
                turn_doc.junme,
                turn_mat.junme
            );
        }
        let Some(cand) = turn_mat
            .candidates
            .iter()
            .find(|c| c.label == turn_doc.recommended)
        else {
            bail!(
                "{at}: recommended label {:?} is not one of the offered candidates",
                turn_doc.recommended
            );
        };
        if turn_doc.numbers != cand.numbers {
            bail!(
                "{at}: engine numbers were altered (reply {:?} vs material {:?}) \
                 — hallucinated numbers are rejected",
                turn_doc.numbers,
                cand.numbers
            );
        }
        if turn_doc.explanation_zh.trim().is_empty() {
            bail!("{at}: empty explanation");
        }
    }

    // Authoritative identity comes from the material, never the reply.
    doc.game_id = material.game_id.clone();
    doc.seat = material.seat;

    // A reply that explains turns must open with the game-level summary
    // (part of the prompt contract since D-0012). Old caches without the
    // field still load — this check only runs on fresh replies.
    if !doc.turns.is_empty() {
        let ok = doc
            .overview
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty());
        if !ok {
            bail!("overview missing or empty — the reply must open with a game-level summary");
        }
    }
    Ok(doc)
}

/// Salvage-mode variant used by the pipeline: instead of one bad turn
/// voiding the whole reply, each turn is validated independently and
/// only the offending turns are dropped (with the reason collected for
/// the caller to surface). Every surviving turn passed the exact same
/// checks as [`parse_review_doc`] — D-0005 is not relaxed, one sloppy
/// echo just no longer costs the other eleven turns. Turns are matched
/// to material by junme, so a merged/missing turn cannot shift
/// alignment. Fails only when *nothing* survives or there is no overview.
pub fn parse_review_doc_lenient(
    content: &str,
    material: &ReviewMaterial,
) -> Result<(ReviewDoc, Vec<String>)> {
    let json_span = extract_json_span(content).context("no JSON object found in LLM reply")?;
    let raw: ReviewDoc = serde_json::from_str(json_span)
        .with_context(|| "LLM reply is not valid JSON for a review document")?;

    let mut turns = Vec::new();
    let mut rejected = Vec::new();
    for turn_doc in raw.turns {
        let Some(turn_mat) = material.turns.iter().find(|t| t.junme == turn_doc.junme) else {
            rejected.push(format!(
                "junme {}: not part of this material (reply {:?})",
                turn_doc.junme, turn_doc.recommended
            ));
            continue;
        };
        let at = format!("junme {}", turn_mat.junme);
        let ok = turn_mat
            .candidates
            .iter()
            .find(|c| c.label == turn_doc.recommended)
            .is_some_and(|c| turn_doc.numbers == c.numbers)
            && !turn_doc.explanation_zh.trim().is_empty();
        if ok {
            turns.push(turn_doc);
        } else {
            let cand = turn_mat
                .candidates
                .iter()
                .find(|c| c.label == turn_doc.recommended);
            let reason = match cand {
                None => format!(
                    "{at}: recommended {:?} is not an offered candidate",
                    turn_doc.recommended
                ),
                Some(c) if turn_doc.numbers != c.numbers => format!(
                    "{at}: engine numbers were altered (reply {:?} vs material {:?})",
                    turn_doc.numbers, c.numbers
                ),
                _ => format!("{at}: empty explanation"),
            };
            rejected.push(reason);
        }
    }
    if turns.is_empty() {
        bail!(
            "no turn survived validation ({} rejected) — degrading to a numbers-only review",
            rejected.len()
        );
    }
    let doc = ReviewDoc {
        game_id: material.game_id.clone(),
        seat: material.seat,
        model: raw.model,
        created_at: raw.created_at,
        overview: raw.overview,
        turns,
    };
    if !doc
        .overview
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        bail!("overview missing or empty — the reply must open with a game-level summary");
    }
    Ok((doc, rejected))
}

/// Trust-passthrough review parse (current stage, D-0013): keep the
/// LLM's reply verbatim — no turn dropping, no number cross-checking.
/// Only the irreducible minimum happens: JSON extraction, identity
/// stamping from the material (game_id/seat are ours to assert, the
/// model has no say), and a non-empty turn list. Numbers rendered by
/// the frontend still come from this doc as received.
pub fn parse_review_doc_trust(content: &str, material: &ReviewMaterial) -> Result<ReviewDoc> {
    let json_span = extract_json_span(content).context("no JSON object found in LLM reply")?;
    let mut doc: ReviewDoc = serde_json::from_str(json_span)
        .with_context(|| "LLM reply is not valid JSON for a review document")?;
    if doc.turns.is_empty() {
        bail!("reply carried no turns — nothing to keep");
    }
    doc.game_id = material.game_id.clone();
    doc.seat = material.seat;
    Ok(doc)
}

/// Trust-passthrough single-turn parse (current stage, D-0013) — the
/// live deep-dive twin of [`parse_review_doc_trust`].
pub fn parse_turn_doc_trust(
    content: &str,
    material: &crate::tutorial::material::TurnMaterial,
) -> Result<crate::tutorial::material::TurnDoc> {
    let json_span = extract_json_span(content).context("no JSON object found in LLM reply")?;
    let mut doc: crate::tutorial::material::TurnDoc = serde_json::from_str(json_span)
        .with_context(|| "LLM reply is not valid JSON for a turn document")?;
    if doc.explanation_zh.trim().is_empty() && doc.headline.is_none() {
        bail!("reply carried neither explanation nor headline — nothing to keep");
    }
    // junme is ours to assert; the model may echo it loosely.
    doc.junme = material.junme;
    Ok(doc)
}

/// The substring from the first `{` to the last `}` — tolerates models
/// that wrap the object in prose or code fences. `None` when no brace
/// pair exists.
fn extract_json_span(content: &str) -> Option<&str> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    (end > start).then(|| &content[start..=end])
}

/// Parse and validate a single-turn deep-dive reply against its material
/// (the live coach variant, D-0008 §4). Same rules as
/// [`parse_review_doc`], minus the array/overview contract: junme must
/// match, `recommended` must be an offered candidate, `numbers` must
/// equal that candidate's engine numbers exactly, and the explanation
/// must be non-empty.
pub fn parse_turn_doc(
    content: &str,
    material: &crate::tutorial::material::TurnMaterial,
) -> Result<crate::tutorial::material::TurnDoc> {
    let json_span = extract_json_span(content).context("no JSON object found in LLM reply")?;
    let mut doc: crate::tutorial::material::TurnDoc = serde_json::from_str(json_span)
        .with_context(|| "LLM reply is not valid JSON for a turn document")?;

    if doc.junme != material.junme {
        bail!(
            "junme {} does not match material {}",
            doc.junme,
            material.junme
        );
    }
    let Some(cand) = material
        .candidates
        .iter()
        .find(|c| c.label == doc.recommended)
    else {
        bail!(
            "recommended label {:?} is not one of the offered candidates",
            doc.recommended
        );
    };
    if doc.numbers != cand.numbers {
        bail!(
            "engine numbers were altered (reply {:?} vs material {:?}) \
             — hallucinated numbers are rejected",
            doc.numbers,
            cand.numbers
        );
    }
    if doc.explanation_zh.trim().is_empty() {
        bail!("empty explanation");
    }
    Ok(std::mem::take(&mut doc))
}

/// True when the engine numbers are structurally present-but-unset —
/// exposed for the pipeline to decide whether a turn is reviewable at
/// all (a default [`EngineNumbers`] means the engine had nothing to say,
/// so there is nothing trustworthy to narrate).
pub fn is_blank_numbers(n: &EngineNumbers) -> bool {
    *n == EngineNumbers::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tutorial::material::{CandidateNumbers, TurnDoc, TurnMaterial};

    fn material() -> ReviewMaterial {
        ReviewMaterial {
            game_id: "01JTESTGAME".into(),
            seat: 2,
            num_players: 4,
            player_name: None,
            turns: vec![TurnMaterial {
                junme: 6,
                tehai: vec!["5s".into()],
                draw: "6m".into(),
                actual: "dahai:5s".into(),
                candidates: vec![CandidateNumbers {
                    label: "dahai:5s".into(),
                    prob: 0.44,
                    numbers: EngineNumbers {
                        shanten: 0,
                        win_rate: Some(0.42),
                        deal_in_rate: Some(0.09),
                        avg_score: Some(5200),
                    },
                }],
                context: None,
            }],
        }
    }

    fn reply_json(numbers: &str, overview: Option<&str>) -> String {
        // Identity fields are deliberately wrong ("whatever" / seat 9) to
        // prove the parser takes them from the material, not the reply.
        let overview = match overview {
            Some(o) => format!("\"overview\":\"{o}\","),
            None => String::new(),
        };
        format!(
            r#"{{"game_id":"whatever","seat":9,{overview}"turns":[{{"junme":6,"recommended":"dahai:5s","numbers":{numbers},"explanation_zh":"切5s后听牌，待牌含dora 4s，进攻价值高。","headline":"进攻"}}]}}"#
        )
    }

    #[test]
    fn parses_faithful_reply_and_takes_identity_from_material() {
        let raw = format!(
            "好的，复盘如下：\n{}\n希望对你有帮助",
            reply_json(
                "{\"shanten\":0,\"win_rate\":0.42,\"deal_in_rate\":0.09,\"avg_score\":5200}",
                Some("前半程推进顺利，第6巡起听牌后打法偏保守。")
            )
        );
        let doc = parse_review_doc(&raw, &material()).unwrap();
        assert_eq!(doc.game_id, "01JTESTGAME");
        assert_eq!(doc.seat, 2);
        assert_eq!(
            doc.overview.as_deref(),
            Some("前半程推进顺利，第6巡起听牌后打法偏保守。")
        );
        assert_eq!(doc.turns.len(), 1);
        assert_eq!(doc.turns[0].recommended, "dahai:5s");
        assert!(doc.turns[0].explanation_zh.contains("切5s"));
    }

    #[test]
    fn rejects_missing_overview() {
        let raw = reply_json(
            "{\"shanten\":0,\"win_rate\":0.42,\"deal_in_rate\":0.09,\"avg_score\":5200}",
            None,
        );
        let err = parse_review_doc(&raw, &material()).unwrap_err();
        assert!(
            format!("{err:#}").contains("overview"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn rejects_altered_numbers() {
        // The LLM "improved" 0.42 to 0.55 — the core hallucination case.
        let raw = reply_json(
            "{\"shanten\":0,\"win_rate\":0.55,\"deal_in_rate\":0.09,\"avg_score\":5200}",
            Some("总评"),
        );
        let err = parse_review_doc(&raw, &material()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("altered"), "unexpected error: {msg}");
    }

    #[test]
    fn rejects_unknown_recommendation_and_count_mismatch() {
        // Label not offered by the material.
        let raw = reply_json(
            "{\"shanten\":0,\"win_rate\":0.42,\"deal_in_rate\":0.09,\"avg_score\":5200}",
            Some("总评"),
        )
        .replace("dahai:5s", "reach");
        assert!(parse_review_doc(&raw, &material())
            .unwrap_err()
            .to_string()
            .contains("not one of the offered candidates"));

        // Zero turns vs one material turn.
        let empty = "{\"game_id\":\"x\",\"seat\":0,\"turns\":[],\"overview\":\"x\"}";
        assert!(parse_review_doc(empty, &material())
            .unwrap_err()
            .to_string()
            .contains("turn count mismatch"));
    }

    #[test]
    fn turn_doc_round_trips_through_serde() {
        // The stored/cached shape must survive a JSON round-trip so old
        // caches keep loading after format tweaks (serde defaults).
        let doc = ReviewDoc {
            turns: vec![TurnDoc {
                junme: 6,
                recommended: "dahai:5s".into(),
                numbers: EngineNumbers {
                    shanten: 0,
                    win_rate: Some(0.42),
                    deal_in_rate: None,
                    avg_score: Some(5200),
                },
                explanation_zh: "讲解".into(),
                headline: Some("进攻".into()),
            }],
            ..Default::default()
        };
        let back: ReviewDoc = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(back, doc);
    }

    // --- live deep-dive (parse_turn_doc) ---

    fn turn_material() -> TurnMaterial {
        TurnMaterial {
            junme: 5,
            tehai: vec!["5s".into()],
            draw: "6m".into(),
            actual: "dahai:5s".into(),
            candidates: vec![CandidateNumbers {
                label: "dahai:5s".into(),
                prob: 0.9,
                numbers: EngineNumbers {
                    shanten: 1,
                    win_rate: Some(0.31),
                    deal_in_rate: Some(0.08),
                    avg_score: Some(3900),
                },
            }],
            context: None,
        }
    }

    fn turn_reply(numbers: &str) -> String {
        format!(
            r#"{{"junme":5,"recommended":"dahai:5s","numbers":{numbers},"explanation_zh":"切5s保进张，两面听牌机会更大。","headline":"保进张"}}"#
        )
    }

    #[test]
    fn trust_parse_keeps_everything_verbatim() {
        // Numbers that would fail strict cross-checking (altered digits,
        // invented candidate) survive untouched under D-0013.
        let raw = concat!(
            "前置废话
{\"overview\":\"总评\",\"game_id\":\"whatever\",\"seat\":9,",
            "\"turns\":[{\"junme\":6,\"recommended\":\"dahai:9z\",",
            "\"numbers\":{\"shanten\":0,\"win_rate\":0.99,\"deal_in_rate\":null,\"avg_score\":null},",
            "\"explanation_zh\":\"讲解\",\"headline\":null}]}
后置废话"
        );
        let doc = parse_review_doc_trust(raw, &material()).unwrap();
        assert_eq!(
            doc.game_id, "01JTESTGAME",
            "identity is stamped from the material"
        );
        assert_eq!(doc.seat, 2);
        assert_eq!(doc.turns.len(), 1);
        assert_eq!(doc.turns[0].recommended, "dahai:9z");
        assert_eq!(doc.turns[0].numbers.win_rate, Some(0.99));

        // Empty replies are still useless.
        let empty = "{\"turns\":[],\"overview\":\"x\"}";
        assert!(parse_review_doc_trust(empty, &material()).is_err());
    }

    #[test]
    fn trust_turn_parse_stamps_our_junme() {
        let raw = r#"{"junme": 999, "recommended": "dahai:5s", "numbers": {"shanten": 1},
                      "explanation_zh": "讲解", "headline": "标题"}"#;
        let doc = parse_turn_doc_trust(raw, &turn_material()).unwrap();
        assert_eq!(doc.junme, 5, "junme is ours to assert");
        assert_eq!(doc.recommended, "dahai:5s");
    }

    #[test]
    fn parses_faithful_turn_reply() {
        let raw = format!(
            "讲解如下：\n{}\n以上",
            turn_reply(
                "{\"shanten\":1,\"win_rate\":0.31,\"deal_in_rate\":0.08,\"avg_score\":3900}"
            )
        );
        let doc = parse_turn_doc(&raw, &turn_material()).unwrap();
        assert_eq!(doc.junme, 5);
        assert_eq!(doc.recommended, "dahai:5s");
        assert_eq!(doc.headline.as_deref(), Some("保进张"));
    }

    #[test]
    fn rejects_turn_reply_with_altered_numbers() {
        let raw =
            turn_reply("{\"shanten\":1,\"win_rate\":0.9,\"deal_in_rate\":0.08,\"avg_score\":3900}");
        assert!(parse_turn_doc(&raw, &turn_material()).is_err());
    }

    #[test]
    fn rejects_turn_reply_with_wrong_junme_or_label() {
        let wrong_junme = turn_reply(
            "{\"shanten\":1,\"win_rate\":0.31,\"deal_in_rate\":0.08,\"avg_score\":3900}",
        )
        .replace("\"junme\":5", "\"junme\":4");
        assert!(parse_turn_doc(&wrong_junme, &turn_material()).is_err());

        let wrong_label = turn_reply(
            "{\"shanten\":1,\"win_rate\":0.31,\"deal_in_rate\":0.08,\"avg_score\":3900}",
        )
        .replace("dahai:5s", "dahai:9p");
        assert!(parse_turn_doc(&wrong_label, &turn_material()).is_err());
    }
}
