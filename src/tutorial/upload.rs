//! Upload zone — the **only** path tutorial data takes out of this
//! machine (D-0007).
//!
//! Responsibilities, in call order:
//! 1. [`outbound_allowed`] — the consent gate: feature on + user opted
//!    in to uploads + endpoint configured. No network call may happen
//!    in this module's callers without this passing first.
//! 2. [`redact`] — strip identity data (player names, and any future
//!    platform identifiers) from the material.
//! 3. [`build_messages`] — render the redacted material into the chat
//!    prompt, including the JSON output contract that
//!    [`crate::tutorial::download`] parses back.
//!
//! This module owns no I/O; it produces [`ChatMessage`]s for the
//! provider. Keeping it pure makes the "what leaves this machine"
//! question answerable by reading one file.

use crate::config::TutorialConfig;
use crate::tutorial::material::ReviewMaterial;
use crate::tutorial::provider::ChatMessage;

/// The outbound gate. Kept as a named function (rather than calling
/// `cfg.outbound_allowed()` at call sites) so the audit trail reads
/// "everything leaves via upload::outbound_allowed".
pub fn outbound_allowed(cfg: &TutorialConfig) -> bool {
    cfg.outbound_allowed()
}

/// Strip identity data from the material. The reviewed hand itself is
/// not identifying; names and (when they land) platform ids / room ids
/// are. Extend this function whenever a new identity-ish field is added
/// to [`ReviewMaterial`] — the compiler won't force you, the README's
/// invariant list will.
pub fn redact(material: &ReviewMaterial) -> ReviewMaterial {
    let mut m = material.clone();
    m.player_name = None;
    m
}

/// Render the (already redacted) material into the chat prompt.
///
/// Numbers are rendered with `{:?}` (round-trip float formatting) so the
/// download zone can cross-check the LLM's echo with exact equality —
/// see `download::parse_review_doc`.
pub fn build_messages(material: &ReviewMaterial) -> Vec<ChatMessage> {
    vec![
        ChatMessage::system(system_prompt()),
        ChatMessage::user(render_material(material)),
    ]
}

fn system_prompt() -> String {
    // The output contract mirrors `material::ReviewDoc`'s serde shape.
    // If that type changes, change this string in the same commit.
    "你是麻将复盘教练。用户会给出一份对局数据，其中所有数字（向听 shanten、\
     和率 win_rate、铳率 deal_in_rate、期望打点 avg_score）都由本地分析引擎预先算好。\
     你的职责：为每一巡写一段面向初中级玩家的中文讲解（解释关键取舍，60-120字），\
     并推荐一条候选路线。铁律：数字必须原样引用，禁止计算、换算或修改任何数字。\
     只输出一个 JSON 对象，不要输出其他文字，结构如下：\n\
     {\"game_id\": \"<原样>\", \"seat\": <原样>, \"turns\": [{\"junme\": <巡号>, \
     \"recommended\": \"<候选label>\", \"numbers\": {\"shanten\": <int>, \
     \"win_rate\": <float|null>, \"deal_in_rate\": <float|null>, \"avg_score\": <int|null>}, \
     \"explanation_zh\": \"<讲解>\", \"headline\": \"<一句话结论|null>\"}]}\n\
     turns 数组必须与输入的巡数一一对应、顺序一致；numbers 必须逐字段复制推荐候选的数字。"
        .to_string()
}

fn render_material(material: &ReviewMaterial) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "对局 {}（{}人，座位 {}），共 {} 个决策点：\n",
        material.game_id,
        material.num_players,
        material.seat,
        material.turns.len()
    ));
    for turn in &material.turns {
        out.push_str(&format!(
            "\n=== 巡 {} ===\n手牌: {} 摸: {}\n实际选择: {}\n候选:\n",
            turn.junme,
            turn.tehai.join(" "),
            turn.draw,
            turn.actual
        ));
        for (i, cand) in turn.candidates.iter().enumerate() {
            out.push_str(&format!(
                "  {}. {} (p={}) 向听{} 和率{} 铳率{} 打点{}\n",
                i + 1,
                cand.label,
                cand.prob,
                cand.numbers.shanten,
                fmt_opt(cand.numbers.win_rate),
                fmt_opt(cand.numbers.deal_in_rate),
                fmt_opt(cand.numbers.avg_score),
            ));
        }
    }
    out
}

/// Render an optional number for the prompt. `{:?}` on the inner value
/// keeps floats round-trip exact; `None` renders as `null` to match the
/// JSON contract the LLM is told to echo.
fn fmt_opt<T: std::fmt::Debug>(v: Option<T>) -> String {
    match v {
        Some(v) => format!("{v:?}"),
        None => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tutorial::material::{CandidateNumbers, EngineNumbers, TurnMaterial};

    fn sample_material() -> ReviewMaterial {
        ReviewMaterial {
            game_id: "01JTESTGAME".into(),
            seat: 2,
            num_players: 4,
            player_name: Some("HcPlu".into()),
            turns: vec![TurnMaterial {
                junme: 6,
                tehai: vec![
                    "1m", "2m", "3m", "4m", "5m", "6m", "8m", "8m", "3p", "4p", "5p", "2s", "3s",
                    "5s",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                draw: "6m".into(),
                actual: "dahai:5s".into(),
                candidates: vec![
                    CandidateNumbers {
                        label: "dahai:5s".into(),
                        prob: 0.44,
                        numbers: EngineNumbers {
                            shanten: 0,
                            win_rate: Some(0.42),
                            deal_in_rate: Some(0.09),
                            avg_score: Some(5200),
                        },
                    },
                    CandidateNumbers {
                        label: "dahai:8m".into(),
                        prob: 0.21,
                        numbers: EngineNumbers {
                            shanten: 1,
                            win_rate: Some(0.31),
                            deal_in_rate: Some(0.08),
                            avg_score: Some(3900),
                        },
                    },
                ],
            }],
        }
    }

    #[test]
    fn redact_strips_identity() {
        let m = sample_material();
        assert_eq!(m.player_name.as_deref(), Some("HcPlu"));
        let r = redact(&m);
        assert_eq!(r.player_name, None);
        // Everything else survives untouched.
        assert_eq!(r.game_id, m.game_id);
        assert_eq!(r.turns, m.turns);
    }

    #[test]
    fn build_messages_carries_contract_and_every_turn() {
        let msgs = build_messages(&redact(&sample_material()));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        // The JSON contract must name the fields download parses.
        for key in ["explanation_zh", "recommended", "numbers", "win_rate"] {
            assert!(msgs[0].content.contains(key), "system prompt lacks {key}");
        }
        assert_eq!(msgs[1].role, "user");
        let user = &msgs[1].content;
        assert!(user.contains("01JTESTGAME"));
        assert!(user.contains("巡 6"));
        assert!(user.contains("dahai:5s"));
        // Identity must not survive into the prompt.
        assert!(!user.contains("HcPlu"));
        // Numbers round-trip-render (0.42 not 0.42000...).
        assert!(user.contains("和率0.42"));
    }
}
