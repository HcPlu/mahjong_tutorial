// Commentary rules engine (M2 template layer, D-0008): deterministic,
// local, free. Turns the latest analysis/bot/snapshot data into at most
// one coaching line per decision.
//
// D-0005 discipline: rule parameters are taken verbatim from event
// fields (AnalysisResult / meta.show / snapshot) — the only arithmetic
// allowed is display formatting (percent rounding, list slicing), never
// new analysis.

import { tileIdx } from '@/lib/tileIdx'
import type { AnalysisResult, GameStateSnapshot, ShowMeta } from '@/types'

export type CommentaryRuleId = 'defence' | 'tenpai' | 'bot-confident'

export type CommentaryLine = {
  ruleId: CommentaryRuleId
  /** Full i18n key for the sentence. */
  i18nKey: string
  params: Record<string, string | number>
  /** Mjai tiles rendered inline with the sentence (mahgen). */
  tiles: string[]
}

const BOT_CONFIDENT_PROB = 0.6
const BOT_CONFIDENT_GAP = 0.3
const MAX_WAIT_TILES = 3

/** Evaluate all rules and return the single highest-priority hit
 *  (defence > tenpai > bot confidence), or null. */
export function evaluate(
  analysis: AnalysisResult | null,
  show: ShowMeta | null,
  snapshot: GameStateSnapshot | null,
): CommentaryLine | null {
  return defenceRule(analysis, snapshot) ?? tenpaiRule(analysis) ?? botConfidentRule(show)
}

/** R-DF1: an opponent is in riichi and the engine names a safest tile. */
function defenceRule(
  analysis: AnalysisResult | null,
  snapshot: GameStateSnapshot | null,
): CommentaryLine | null {
  if (!analysis || !snapshot) return null
  const ourSeat = snapshot.our_seat
  const threat = snapshot.players.some(
    (p) => p.riichi_declared && (ourSeat === null || p.seat !== ourSeat),
  )
  if (!threat) return null
  const tile = analysis.best_defence_discard
  if (!tile) return null
  const idx = tileIdx(tile)
  const riskPct =
    idx >= 0 && idx < analysis.mixed_risk.length ? analysis.mixed_risk[idx] : null
  return {
    ruleId: 'defence',
    i18nKey: 'coach.defence',
    params: { tile, p: riskPct === null ? '—' : riskPct.toFixed(1) },
    tiles: [tile],
  }
}

/** R-EF2: we are tenpai between draws — say what we wait on. */
function tenpaiRule(analysis: AnalysisResult | null): CommentaryLine | null {
  if (!analysis || analysis.state !== 'wait13' || analysis.shanten !== 0) return null
  const h13 = analysis.hand13
  if (!h13 || h13.waits.length === 0) return null
  const tiles = h13.waits.slice(0, MAX_WAIT_TILES).map((w) => w.tile)
  return {
    ruleId: 'tenpai',
    i18nKey: 'coach.tenpai',
    params: { tiles: tiles.join(' '), n: h13.waits_total },
    tiles,
  }
}

/** R-BT1: the bot's top choice dominates its second. */
function botConfidentRule(show: ShowMeta | null): CommentaryLine | null {
  if (!show || show.items.length === 0) return null
  const first = show.items[0]
  const prob = first.prob
  if (prob === undefined || prob < BOT_CONFIDENT_PROB) return null
  const second = show.items[1]?.prob
  if (second !== undefined && prob - second < BOT_CONFIDENT_GAP) return null
  return {
    ruleId: 'bot-confident',
    i18nKey: 'coach.bot_confident',
    params: { action: first.label ?? '', prob: Math.round(prob * 100) },
    tiles: first.pais ?? [],
  }
}
