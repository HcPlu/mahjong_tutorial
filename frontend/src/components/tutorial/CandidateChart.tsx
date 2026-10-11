// Discard-candidate comparison for the replay page: the engine's top
// maintain candidates with the ranking score, win-rate / deal-in-risk
// bars, plus badges for the student's actual pick and the engine's
// attack-best. A bot's played tile outside the engine's top rows swaps
// in for the last row, tagged with its true engine rank. Every number is
// taken verbatim from the local
// AnalysisResult — no math beyond the engine's percent → 0..1 display
// conversion. Dimensions with no signal in the current set (e.g. win
// rate at 3-shanten, risk with nobody tenpai) collapse to "—" instead
// of painting every row with the same "0%".

import { useTranslation } from 'react-i18next'

import { Mahgen } from '@/components/Mahgen'
import { mjaiToMahgen, tileIdx } from '@/lib/tileIdx'
import type { BotRefSource, BotRefTurn } from '@/components/tutorial/types'
import type { AnalysisResult, DiscardCandidate } from '@/types'

type Props = {
  analysis: AnalysisResult
  /** The student's actual discard this turn (mjai tile), when known. */
  actual: string | null
  /** Bot reference for this turn (offline replay), when generated. */
  botTurn?: BotRefTurn | null
  /** Bot answers recorded while the game was played (D-0017), when the
   *  game ran with a bot. Same shape as `botTurn` — the point is that
   *  the two bots may differ. */
  liveTurn?: BotRefTurn | null
}

const MAX_ROWS = 5

export function CandidateChart({ analysis, actual, botTurn, liveTurn }: Props) {
  const full = analysis.hand14?.maintain ?? []
  if (full.length === 0) return null
  // The engine's own ranking (1-based) of every maintain candidate — a
  // swapped-in row must still tell the truth about where the engine put it.
  const rankOf = new Map(full.map((c, i) => [c.discard, i + 1]))
  // Rows are the engine's top candidates, except that a bot's played tile
  // outside them swaps in for the last row (working upward for a second
  // bot) — otherwise the chosen badge would have no row to live on.
  const rows: { cand: DiscardCandidate; engineRank: number | null }[] = full
    .slice(0, MAX_ROWS)
    .map((cand) => ({ cand, engineRank: null }))
  const missing: string[] = []
  for (const turn of [botTurn, liveTurn]) {
    const tile = turn?.picks.find((p) => p.chosen)?.tile ?? forcedDiscard(turn)
    if (tile && !rows.some((r) => r.cand.discard === tile) && !missing.includes(tile)) {
      missing.push(tile)
    }
  }
  let swapAt = rows.length - 1
  for (const tile of missing) {
    // Only maintain discards have engine rows; a backwards pick stays hidden.
    const cand = full.find((c) => c.discard === tile)
    if (!cand) continue
    if (rows.length < MAX_ROWS) rows.push({ cand, engineRank: rankOf.get(tile) ?? null })
    else if (swapAt >= 0) rows[swapAt--] = { cand, engineRank: rankOf.get(tile) ?? null }
  }
  const best = analysis.best_attack_discard
  const risks = rows.map((r) => riskFor(analysis, r.cand.discard))
  // A metric only renders when the current turn gives it signal — all
  // zeroes mean the engine has nothing to say there (far hand / no
  // tenpai threat), and a wall of "0%" reads as broken.
  const winLive = rows.some((r) => r.cand.result.avg_agari_rate > 0)
  const riskLive = risks.some((r) => (r ?? 0) > 0)
  const picks = botTurn?.picks ?? null
  const livePicks = liveTurn?.picks ?? null
  // With two bot sources in play the percentages get source prefixes
  // ("复盘/对局"); with one, the bare label reads cleaner.
  const dual = picks != null && livePicks != null
  // A rule-forced turn has no distribution; the forced tile rides only
  // in the action label ("dahai:5s").
  const forcedTile = forcedDiscard(botTurn)
  const liveForcedTile = forcedDiscard(liveTurn)

  return (
    <div className="space-y-1.5">
      <div className="flex justify-end gap-1">
        <SourceBadge source={botTurn?.source ?? null} />
        <SourceBadge source={liveTurn?.source ?? null} live />
      </div>
      {rows.map(({ cand, engineRank }, i) => {
        const botPick = picks?.find((p) => p.tile === cand.discard) ?? null
        const livePick = livePicks?.find((p) => p.tile === cand.discard) ?? null
        return (
          <CandidateRow
            key={cand.discard}
            cand={cand}
            engineRank={engineRank}
            risk={risks[i]}
            winLive={winLive}
            riskLive={riskLive}
            isActual={actual !== null && cand.discard === actual}
            isBest={best !== null && cand.discard === best}
            botProb={botPick?.prob ?? null}
            isBotTop={picks != null && picks.length > 0 && picks[0].tile === cand.discard}
            isBotChosen={botPick?.chosen === true || forcedTile === cand.discard}
            liveProb={livePick?.prob ?? null}
            isLiveTop={
              livePicks != null && livePicks.length > 0 && livePicks[0].tile === cand.discard
            }
            isLiveChosen={livePick?.chosen === true || liveForcedTile === cand.discard}
            dual={dual}
          />
        )
      })}
    </div>
  )
}

/** A rule-forced turn has no distribution, so the forced tile rides
 *  only in the action label ("dahai:5s"). */
function forcedDiscard(turn: BotRefTurn | null | undefined): string | null {
  return turn?.source === 'rule_forced' && turn.action.startsWith('dahai:')
    ? turn.action.slice('dahai:'.length)
    : null
}

/** Turn-level provenance of the bot's answer — the whole point is that
 *  a rule fast path carries no probability distribution to trust. The
 *  live recorder's badge is tinted cyan and worded "对局/…" to keep the
 *  two bots apart. */
function SourceBadge({ source, live }: { source: BotRefSource | null; live?: boolean }) {
  const { t } = useTranslation()
  if (source == null || source === 'no_answer') return null
  const style =
    source === 'model'
      ? live
        ? 'border-sky-500/40 text-sky-600 dark:text-sky-400'
        : 'border-border text-muted-foreground'
      : source === 'rule_forced'
        ? 'border-amber-500/40 text-amber-600 dark:text-amber-400'
        : 'border-red-500/40 text-red-600 dark:text-red-400'
  const key = live ? `tutorial.live_bot_source_${source}` : `tutorial.bot_source_${source}`
  return (
    <span className={'rounded border px-1.5 py-0.5 text-[10px] ' + style}>{t(key)}</span>
  )
}

function CandidateRow({
  cand,
  engineRank,
  risk,
  winLive,
  riskLive,
  isActual,
  isBest,
  botProb,
  isBotTop,
  isBotChosen,
  liveProb,
  isLiveTop,
  isLiveChosen,
  dual,
}: {
  cand: DiscardCandidate
  /** Engine rank of a swapped-in row (null = the row is engine top-N as
   *  displayed); tells the reader the row sits below the shown cutoff. */
  engineRank: number | null
  /** Deal-in risk for the discarded tile, 0..1 (mixed risk / 100). */
  risk: number | null
  winLive: boolean
  riskLive: boolean
  isActual: boolean
  isBest: boolean
  /** Bot's probability for this exact tile, when a reference exists. */
  botProb: number | null
  isBotTop: boolean
  /** The action the bot actually played this turn (model pick, guard
   *  override, or rule-forced discard). */
  isBotChosen: boolean
  /** Same trio for the bot recorded during the game (D-0017). */
  liveProb: number | null
  isLiveTop: boolean
  isLiveChosen: boolean
  /** Both bot sources present — percentages get source prefixes. */
  dual: boolean
}) {
  const { t } = useTranslation()
  const r = cand.result
  const win = clamp01(r.avg_agari_rate / 100)

  return (
    <div
      className={
        'flex items-center gap-3 rounded-md border px-2 py-1.5 ' +
        (isActual ? 'border-primary/60 bg-primary/5' : 'border-border')
      }
    >
      <Mahgen seq={mjaiToMahgen([cand.discard])} kind="bot-show" />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-center gap-2 text-xs">
          <span className="text-muted-foreground">{t('tutorial.shanten')}</span>
          <span className="font-mono tabular-nums">{r.shanten}</span>
          <span className="ml-1 text-muted-foreground">{t('tutorial.waits')}</span>
          <span className="font-mono tabular-nums">{r.waits_total}</span>
          {/* The engine's actual ranking criterion — the one number that
              separates candidates that tie on ukeire. */}
          <span className="ml-1 text-muted-foreground">{t('tutorial.score')}</span>
          <span className="font-mono tabular-nums">{r.mixed_waits_score.toFixed(1)}</span>
        </div>
        <div className="flex items-center gap-2">
          {winLive ? (
            <>
              <Bar value={win} className="bg-emerald-500/70" />
              <span className="w-12 shrink-0 text-right font-mono text-xs tabular-nums text-emerald-600 dark:text-emerald-400">
                {(win * 100).toFixed(0)}%
              </span>
            </>
          ) : (
            <DeadMetric label={t('tutorial.win_rate')} />
          )}
          {riskLive ? (
            <>
              <Bar value={risk ?? 0} className="bg-red-500/70" />
              <span className="w-12 shrink-0 text-right font-mono text-xs tabular-nums text-red-600 dark:text-red-400">
                {risk === null ? '—' : `${(risk * 100).toFixed(1)}%`}
              </span>
            </>
          ) : (
            <DeadMetric label={t('tutorial.deal_in_rate')} />
          )}
        </div>
      </div>
      <div className="flex shrink-0 flex-col items-end gap-1">
        {botProb !== null && (
          <span
            className={
              'font-mono text-[10px] tabular-nums ' +
              (isBotChosen
                ? 'font-semibold text-violet-600 dark:text-violet-400'
                : 'text-muted-foreground')
            }
          >
            {dual
              ? t('tutorial.replay_bot_prob', { p: Math.round(botProb * 100) })
              : t('tutorial.bot_prob', { p: Math.round(botProb * 100) })}
          </span>
        )}
        {liveProb !== null && (
          <span
            className={
              'font-mono text-[10px] tabular-nums ' +
              (isLiveChosen
                ? 'font-semibold text-sky-600 dark:text-sky-400'
                : 'text-muted-foreground')
            }
          >
            {t('tutorial.live_bot_prob', { p: Math.round(liveProb * 100) })}
          </span>
        )}
        {isActual && (
          <span className="rounded bg-primary/15 px-1.5 py-0.5 text-[10px] font-medium text-primary">
            {t('tutorial.actual_pick')}
          </span>
        )}
        {isBest && (
          <span className="rounded bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
            {t('tutorial.engine_best')}
          </span>
        )}
        {engineRank != null && (
          <span className="rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground">
            {t('tutorial.engine_rank', { rank: engineRank })}
          </span>
        )}
        {isLiveChosen ? (
          <span className="rounded bg-sky-600 px-1.5 py-0.5 text-[10px] font-medium text-white">
            {t('tutorial.live_bot_chosen')}
          </span>
        ) : (
          isLiveTop && (
            <span className="rounded bg-sky-500/15 px-1.5 py-0.5 text-[10px] font-medium text-sky-600 dark:text-sky-400">
              {t('tutorial.live_bot_pick')}
            </span>
          )
        )}
        {isBotChosen ? (
          <span className="rounded bg-violet-600 px-1.5 py-0.5 text-[10px] font-medium text-white">
            {t('tutorial.bot_chosen')}
          </span>
        ) : (
          /* Rank-1 candidate only gets its own badge when the bot
           * played something else (guard override / sampling) —
           * otherwise the chosen badge already says it. */
          isBotTop && (
            <span className="rounded bg-violet-500/15 px-1.5 py-0.5 text-[10px] font-medium text-violet-600 dark:text-violet-400">
              {t('tutorial.bot_pick')}
            </span>
          )
        )}
      </div>
    </div>
  )
}

/** Collapsed metric: label + em dash, no bar — the engine produced no
 *  signal for this dimension on this turn. */
function DeadMetric({ label }: { label: string }) {
  return (
    <span className="flex w-[9.5rem] shrink-0 items-center gap-1.5 text-xs text-muted-foreground/60">
      <span>{label}</span>
      <span className="font-mono">—</span>
    </span>
  )
}

function Bar({ value, className }: { value: number; className: string }) {
  return (
    <div className="h-2 w-24 shrink-0 overflow-hidden rounded-sm bg-muted">
      <div
        className={'h-full rounded-sm ' + className}
        style={{ width: `${Math.round(clamp01(value) * 100)}%` }}
      />
    </div>
  )
}

function riskFor(analysis: AnalysisResult, discard: string): number | null {
  const idx = tileIdx(discard)
  if (idx < 0 || idx >= analysis.mixed_risk.length) return null
  const pct = analysis.mixed_risk[idx]
  return pct > 0 ? pct / 100 : 0
}

function clamp01(v: number): number {
  return Math.max(0, Math.min(1, v))
}
