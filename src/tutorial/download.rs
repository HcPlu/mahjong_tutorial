//! Download zone — the **only** path cloud data takes into this app
//! (D-0007).
//!
//! [`parse_review_doc`] takes the raw assistant message and the material
//! it was generated from, and returns a [`ReviewDoc`] only if the reply
//! is structurally valid **and** every number it echoes matches the
//! engine numbers of the material exactly (the D-0005 anti-hallucination
//! guard). Any mismatch is an error the caller degrades on — a missing
//! explanation is better than a wrong one.

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
            }],
        }
    }

    fn reply_json(numbers: &str) -> String {
        // Identity fields are deliberately wrong ("whatever" / seat 9) to
        // prove the parser takes them from the material, not the reply.
        format!(
            r#"{{"game_id":"whatever","seat":9,"turns":[{{"junme":6,"recommended":"dahai:5s","numbers":{numbers},"explanation_zh":"切5s后听牌，待牌含dora 4s，进攻价值高。","headline":"进攻"}}]}}"#
        )
    }

    #[test]
    fn parses_faithful_reply_and_takes_identity_from_material() {
        let raw = format!(
            "好的，复盘如下：\n{}\n希望对你有帮助",
            reply_json(
                "{\"shanten\":0,\"win_rate\":0.42,\"deal_in_rate\":0.09,\"avg_score\":5200}"
            )
        );
        let doc = parse_review_doc(&raw, &material()).unwrap();
        assert_eq!(doc.game_id, "01JTESTGAME");
        assert_eq!(doc.seat, 2);
        assert_eq!(doc.turns.len(), 1);
        assert_eq!(doc.turns[0].recommended, "dahai:5s");
        assert!(doc.turns[0].explanation_zh.contains("切5s"));
    }

    #[test]
    fn rejects_altered_numbers() {
        // The LLM "improved" 0.42 to 0.55 — the core hallucination case.
        let raw = reply_json(
            "{\"shanten\":0,\"win_rate\":0.55,\"deal_in_rate\":0.09,\"avg_score\":5200}",
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
        )
        .replace("dahai:5s", "reach");
        assert!(parse_review_doc(&raw, &material())
            .unwrap_err()
            .to_string()
            .contains("not one of the offered candidates"));

        // Zero turns vs one material turn.
        let empty = "{\"game_id\":\"x\",\"seat\":0,\"turns\":[]}";
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
}
