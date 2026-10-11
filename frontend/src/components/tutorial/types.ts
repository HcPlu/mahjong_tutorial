// TS mirrors of the tutorial backend types (`src/tutorial/material.rs` /
// `assemble.rs`). Kept here instead of `types.ts` so the fork-owned
// schema lives with the fork-owned UI (upstream `types.ts` stays
// untouched). Wire-compatible by serde defaults where noted.

import type { AnalysisResult, GameStateSnapshot, MahgenView } from '@/types'

export type ReviewCoverage = 'key' | 'all'

export type EngineNumbers = {
  shanten: number
  win_rate: number | null
  deal_in_rate: number | null
  avg_score: number | null
}

export type TurnDoc = {
  junme: number
  recommended: string
  numbers: EngineNumbers
  explanation_zh: string
  headline: string | null
}

/** Payload of the `tutorial-live-turn` event (D-0014): one auto-generated
 *  (or cache-replayed) live deep-dive, broadcast the moment it becomes
 *  available. */
export type TutorialLiveEvent = {
  session_key: string
  junme: number
  doc: TurnDoc
  model: string | null
}

export type ReviewDoc = {
  game_id: string
  seat: number
  model: string | null
  created_at: string
  /** null only in caches written before the field existed. */
  overview: string | null
  turns: TurnDoc[]
}

export type OutlineDecision = {
  event_index: number
  junme: number
  is_key: boolean
  /** The student's actual discard, mjai tile (e.g. '5s'). */
  actual: string
}

export type ReplayOutline = {
  seat: number
  num_players: number
  total_events: number
  decisions: OutlineDecision[]
}

export type ReplayStep = {
  snapshot: GameStateSnapshot
  mahgen: MahgenView
  analysis: AnalysisResult | null
}

// --- Bot reference (offline replay, D-0012 M4-lite) ---

/** How the bot produced a turn's action (model vs rule fast path).
 *  Wire values from `src/tutorial/material.rs::BotRefSource`. */
export type BotRefSource = 'model' | 'rule_forced' | 'rule_overridden' | 'no_answer'

export type BotRefPick = {
  /** Mjai tile string (e.g. '5s'). */
  tile: string
  label: string
  /** Bot policy probability, 0..1. */
  prob: number
  /** True on the row the bot actually played. Absent in caches written
   *  before the field existed. */
  chosen?: boolean
}

export type BotRefTurn = {
  junme: number
  /** Bot action label ('dahai:5s', 'reach', 'none', ...). */
  action: string
  picks: BotRefPick[]
  /** Whether the action came from the model or a rule fast path.
   *  Absent in caches written before the field existed. */
  source?: BotRefSource | null
}

export type BotRefDoc = {
  game_id: string
  bot: string
  created_at: string
  turns: BotRefTurn[]
}
