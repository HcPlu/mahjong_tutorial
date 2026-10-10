//! Shared data types of the tutorial module: the *teaching material*
//! (input, numbers produced locally) and the *review document* (output,
//! language produced by the LLM around those exact numbers).
//!
//! These types are the contract between the three data zones (see the
//! module README): `upload` renders [`ReviewMaterial`] into a prompt,
//! `download` parses an LLM reply back into a [`ReviewDoc`] whose numbers
//! are cross-checked against the material, and `store` persists
//! [`ReviewDoc`] as the on-disk cache.

use serde::{Deserialize, Serialize};

/// Numbers computed by the local analysis engine (`crate::analysis`) for
/// one decision point / candidate line.
///
/// Invariant (D-0005): every number that reaches the player originates
/// here — the LLM may only echo these values verbatim, never compute or
/// alter them. [`crate::tutorial::download`] enforces this on the way in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineNumbers {
    /// Shanten count of the resulting hand (`-1` = already won/agari
    /// shape, per riichi convention).
    pub shanten: i32,
    /// Estimated win rate of this line from here on, `0.0..=1.0`.
    pub win_rate: Option<f64>,
    /// Estimated deal-in (deal-in-due-to-opponent-tenpai) rate, `0.0..=1.0`.
    pub deal_in_rate: Option<f64>,
    /// Expected score of this line, in hundreds-native points (e.g. 5200).
    pub avg_score: Option<i64>,
}

/// One candidate move at a decision point, with the engine's verdict.
/// `label` is a coarse mjai-style action string, e.g. `dahai:5s` or
/// `reach`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CandidateNumbers {
    pub label: String,
    /// Engine/bot preference for this candidate, `0.0..=1.0`. Ranked
    /// input to the explanation, not something the LLM produces.
    pub prob: f64,
    /// The engine numbers attached to this candidate line.
    pub numbers: EngineNumbers,
}

/// One decision point (junme) of the reviewed seat, as handed to the LLM.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnMaterial {
    pub junme: u32,
    /// Full hand at the decision point, mjai tile notation (`"5s"`),
    /// the drawn tile included (14 tiles for a discard decision).
    pub tehai: Vec<String>,
    /// The tile just drawn (also the last entry of `tehai`).
    pub draw: String,
    /// What the player actually did, same label vocabulary as
    /// [`CandidateNumbers::label`].
    pub actual: String,
    /// Ranked candidates with their engine numbers. Empty when no
    /// engine output is available for this turn.
    pub candidates: Vec<CandidateNumbers>,
}

/// Teaching material for one game, one seat — the input side of the
/// review pipeline. Produced by the M1 pipeline from
/// `crate::history` + `crate::analysis`; nothing here is LLM-generated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviewMaterial {
    /// History ULID of the game being reviewed. Also the store cache key.
    pub game_id: String,
    /// The reviewed seat (0-3).
    pub seat: u8,
    /// 3 or 4 players.
    pub num_players: u8,
    /// Display name of the reviewed player. **Identity data**: must be
    /// stripped by [`crate::tutorial::upload::redact`] before anything
    /// leaves this machine.
    pub player_name: Option<String>,
    pub turns: Vec<TurnMaterial>,
}

// ---------------------------------------------------------------------------
// Output mirror types (review document)
// ---------------------------------------------------------------------------

/// One turn of the generated review: the LLM's Chinese explanation tied
/// back to the engine numbers it is allowed to quote.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnDoc {
    pub junme: u32,
    /// The candidate this explanation recommends — must match a
    /// [`CandidateNumbers::label`] of the corresponding material turn.
    pub recommended: String,
    /// Engine numbers of the recommended line, **copied verbatim from the
    /// material** and cross-checked by [`crate::tutorial::download`].
    pub numbers: EngineNumbers,
    /// The Chinese coaching text for this turn.
    pub explanation_zh: String,
    /// Optional one-line takeaway shown above the full explanation.
    pub headline: Option<String>,
}

/// The persisted artifact of one LLM review — what the save zone writes
/// to `reviews/<game_id>.json` and the frontend renders.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviewDoc {
    /// History ULID (matches [`ReviewMaterial::game_id`]).
    pub game_id: String,
    pub seat: u8,
    /// Model id that served the request, when the provider reported one.
    pub model: Option<String>,
    /// Creation time, RFC 3339 UTC.
    pub created_at: String,
    pub turns: Vec<TurnDoc>,
}

// Manual on purpose: `num_players` defaults to 4-player mahjong, not
// `u8::default()` — the only field here whose default is semantic.
impl Default for ReviewMaterial {
    fn default() -> Self {
        Self {
            game_id: String::new(),
            seat: 0,
            num_players: 4,
            player_name: None,
            turns: Vec::new(),
        }
    }
}
