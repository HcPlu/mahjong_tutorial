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

/// Single-turn variant for the live coach deep dive (D-0008 §4). Same
/// rendering of the decision point, a contract that returns exactly one
/// turn object instead of the whole-game array.
pub fn build_turn_messages(turn: &crate::tutorial::material::TurnMaterial) -> Vec<ChatMessage> {
    vec![
        ChatMessage::system(turn_system_prompt()),
        ChatMessage::user(render_turn(turn)),
    ]
}

/// The live coach's single-turn output contract: one `TurnDoc`-shaped
/// object, no overview, no array.
fn turn_system_prompt() -> String {
    "你是麻将教练，面向初中级玩家教学。给出当前决策点的数据：本地分析引擎预先算好的数字\
     （向听 shanten 为整数、和率 win_rate 与铳率 deal_in_rate 为0-1小数、期望打点 avg_score 为整数点数）、\
     局况与候选切牌。你的职责：解释 bot 首选（实际选择）为什么好——headline 一句话结论（不超过20字）\
     与 explanation_zh（60-120字讲解：取舍逻辑、与其他候选的对比、需要注意的局势）。\
     recommended 填 bot 首选。\n\
     规则背景：立直麻将（日本麻将）。手牌13张、每巡摸1切1，和了形=4面子1雀头（七对/国士例外）；\
     门清听牌可立直（供托1000点）；切牌点炮=铳。记号：dahai:X=切牌X、reach=宣言立直（mjai 协议原样标签）；5mr/5pr/5sr=红宝牌；ESWN=风牌、PFC=三元牌。\
     术语：向听=距听牌还差的有效切牌数；进张/待牌=能推进听牌的摸牌；现物=某家牌河出现过的牌，对该家安全；\
     各家牌河按打出顺序列出，可据其判断对手速度与安全牌。讲解面向初中级玩家：关键术语首次出现时顺带半句白话解释。\n\
     铁律：numbers 字段的数字必须逐字段原样复制；讲解正文里引用数字时用百分数保留1-2位小数，禁止编造任何不在输入中的数字；不得推荐候选之外的打法。\n\
     只输出一个 JSON 对象，不要输出其他文字：\n\
     {\"junme\": <巡号>, \"recommended\": \"<候选label>\", \"numbers\": {\"shanten\": <int>, \
     \"win_rate\": <float|null>, \"deal_in_rate\": <float|null>, \"avg_score\": <int|null>}, \
     \"explanation_zh\": \"<讲解>\", \"headline\": \"<一句话结论>\"}\n\
     numbers 必须逐字段复制推荐候选的数字。"
        .to_string()
}

fn system_prompt() -> String {
    // The output contract mirrors `material::ReviewDoc`'s serde shape.
    // If that type changes, change this string in the same commit.
    "你是麻将复盘教练，面向初中级玩家教学。输入的每个决策点都附有本地分析引擎预先算好的数字\
     （向听 shanten 为整数、和率 win_rate 与铳率 deal_in_rate 为0-1小数、期望打点 avg_score 为整数点数）、\
     局况（风局、本场、分数、已立直家、宝牌指示牌、各家牌河与副露）。你的职责：\n\
     1. overview：2-3句总评本局整体表现（速度、防守、立直时机），可引用具体巡号；\n\
     2. 每巡给出 headline（一句话结论，不超过20字）与 explanation_zh（60-120字讲解：解释关键取舍，\
     对比实际选择与引擎候选，指出更好的打法及其代价，教学口吻）；\n\
     3. recommended：从该巡候选中选一条最优路线；\n\
     4. 若该巡给出 Bot参照（离线 bot 复算的首选与概率），在讲解中对比学员实际选择与 bot 首选\
     （一致时点明共识，分歧时解释 bot 的取舍），并可用各巡牌河信息辅助判断攻防与安全牌。\n\
     规则背景：立直麻将（日本麻将）。手牌13张、每巡摸1切1，和了形=4面子1雀头（七对/国士例外）；\
     门清听牌可立直（供托1000点）；切牌点炮=铳。记号：dahai:X=切牌X、reach=宣言立直（mjai 协议原样标签）；5mr/5pr/5sr=红宝牌；ESWN=风牌、PFC=三元牌。\
     术语：向听=距听牌还差的有效切牌数；进张/待牌=能推进听牌的摸牌；现物=某家牌河出现过的牌，对该家安全；\
     各家牌河按打出顺序列出，可据其判断对手速度与安全牌。讲解面向初中级玩家：关键术语首次出现时顺带半句白话解释。\n\
     铁律：numbers 字段的数字必须逐字段原样复制；讲解正文里引用数字时用百分数保留1-2位小数（如0.03624写作3.6%），禁止编造任何不在输入中的数字；不得推荐候选之外的打法。\
     只输出一个 JSON 对象，不要输出其他文字，结构如下：\n\
     {\"overview\": \"<总评>\", \"game_id\": \"<原样>\", \"seat\": <原样>, \"turns\": [{\"junme\": <巡号>, \
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
        out.push_str(&render_turn(turn));
    }
    out
}

/// Render one decision point. Shared by the whole-game review prompt and
/// the live coach's single-turn prompt (D-0008) so both narrate the same
/// input shape.
pub(crate) fn render_turn(turn: &crate::tutorial::material::TurnMaterial) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "\n=== 巡 {} ===\n手牌: {} 摸: {}\n实际选择: {}\n",
        turn.junme,
        turn.tehai.join(" "),
        turn.draw,
        turn.actual
    ));
    if let Some(ctx) = &turn.context {
        out.push_str(&format!(
            "局况: {}{}局{}本场 分数[{}] 已立直家[{}] 宝牌指示[{}] 我方已立直:{}\n",
            ctx.bakaze,
            ctx.kyoku,
            ctx.honba,
            ctx.scores
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(","),
            ctx.riichi_seats
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(","),
            ctx.dora_indicators.join(" "),
            if ctx.our_riichi { "是" } else { "否" },
        ));
        let seat_labels: Vec<String> = (0..ctx.rivers.len())
            .map(|i| {
                if i == ctx.seat as usize {
                    format!("我方(座位{i})")
                } else {
                    format!("座位{i}")
                }
            })
            .collect();
        if !ctx.rivers.is_empty() {
            let rivers: Vec<String> = ctx
                .rivers
                .iter()
                .zip(&seat_labels)
                .map(|(river, label)| format!("{label}[{}]", river.join(" ")))
                .collect();
            out.push_str(&format!("各家牌河: {}\n", rivers.join(" ")));
        }
        let open_melds: Vec<String> = ctx
            .melds
            .iter()
            .zip(&seat_labels)
            .filter(|(groups, _)| !groups.is_empty())
            .map(|(groups, label)| format!("{label}[{}]", groups.join(" ")))
            .collect();
        if !open_melds.is_empty() {
            out.push_str(&format!("各家副露: {}\n", open_melds.join(" ")));
        }
    }
    out.push_str("候选:\n");
    for (i, cand) in turn.candidates.iter().enumerate() {
        // `prob` is a ranking hint for the model, not a checked number —
        // four decimals reads cleanly and keeps the prompt short.
        out.push_str(&format!(
            "  {}. {} (p={:.4}) 向听{} 和率{} 铳率{} 打点{}\n",
            i + 1,
            cand.label,
            cand.prob,
            cand.numbers.shanten,
            fmt_opt(cand.numbers.win_rate),
            fmt_opt(cand.numbers.deal_in_rate),
            fmt_opt(cand.numbers.avg_score),
        ));
    }
    // Review-only: what the offline bot replay would have done (live
    // materials carry the bot's live probabilities in `p=` instead).
    if let Some(bot) = &turn.bot {
        let picks = bot
            .picks
            .iter()
            .map(|p| format!("{} {:.1}%", p.tile, p.prob * 100.0))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "Bot参照: 首选{}；bot概率: {}\n",
            bot.action, picks
        ));
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
    use crate::tutorial::material::{CandidateNumbers, EngineNumbers, TurnContext, TurnMaterial};

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
                context: Some(TurnContext {
                    bakaze: "S".into(),
                    kyoku: 2,
                    honba: 1,
                    scores: vec![25_000, 18_000, 32_000, 25_000],
                    riichi_seats: vec![3],
                    dora_indicators: vec!["2m".into()],
                    our_riichi: false,
                    seat: 2,
                    rivers: vec![
                        vec!["E".into(), "1m".into()],
                        vec!["S".into(), "9p".into()],
                        vec!["W".into()],
                        vec!["2s".into(), "F".into(), "F".into()],
                    ],
                    melds: vec![vec![], vec!["789s".into()], vec![], vec![]],
                }),
                bot: Some(crate::tutorial::material::BotRefTurn {
                    junme: 6,
                    action: "dahai:8m".into(),
                    picks: vec![
                        crate::tutorial::material::BotRefPick {
                            tile: "8m".into(),
                            label: "Discard".into(),
                            prob: 0.55,
                            chosen: true,
                        },
                        crate::tutorial::material::BotRefPick {
                            tile: "5s".into(),
                            label: "Discard".into(),
                            prob: 0.31,
                            chosen: false,
                        },
                    ],
                    source: None,
                }),
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
        for key in [
            "explanation_zh",
            "recommended",
            "numbers",
            "win_rate",
            "overview",
        ] {
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
        // The ranking hint renders with four fixed decimals.
        assert!(user.contains("(p=0.4400)"));
        // Situational context rides along (no identity in it either).
        assert!(user.contains("局况:"));
        assert!(user.contains("已立直家[3]"));
        // Rivers render per seat with the observer labelled 我方, and the
        // riichi seat's river tiles are visible for 现物 reasoning.
        assert!(user.contains("各家牌河:"));
        assert!(user.contains("座位0[E 1m]"));
        assert!(user.contains("我方(座位2)[W]"));
        assert!(user.contains("座位3[2s F F]"));
        // Only seats with open melds get a line entry.
        assert!(user.contains("各家副露: 座位1[789s]"));
        // The bot reference renders for the review narrator (你 vs bot).
        assert!(user.contains("Bot参照: 首选dahai:8m"));
        assert!(user.contains("8m 55.0%, 5s 31.0%"));
        // The rule anchor rides in the system prompt.
        assert!(msgs[0].content.contains("规则背景"));
        assert!(msgs[0].content.contains("现物"));
        assert!(msgs[0].content.contains("Bot参照"));
    }
}
