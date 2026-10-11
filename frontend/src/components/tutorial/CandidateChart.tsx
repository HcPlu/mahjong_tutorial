// Discard-candidate comparison for the replay page: the engine's top
// maintain candidates with the ranking score, win-rate / deal-in-risk
// bars, plus badges for the student's actual pick and the engine's
// attack-best. Every number is taken verbatim from the local
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
}

const MAX_ROWS = 5

export function CandidateChart({ analysis, actual, botTurn }: Props) {
  const maintain = analysis.hand14?.maintain?.slice(0, MAX_ROWS) ?? []
  if (maintain.length === 0) return null
  const best = analysis.best_attack_discard
  const risks = maintain.map((c) => riskFor(analysis, c.discard))
  // A metric only renders when the current turn gives it signal — all
  // zeroes mean the engine has nothing to say there (far hand / no
  // tenpai threat), and a wall of "0%" reads as broken.
  const winLive = maintain.some((c) => c.result.avg_agari_rate > 0)
  const riskLive = risks.some((r) => (r ?? 0) > 0)
  const picks = botTurn?.picks ?? null
  // A rule-forced turn has no distribution; the forced tile rides only
  // in the action label ("dahai:5s").
  const forcedTile =
    botTurn?.source === 'rule_forced' && botTurn.action.startsWith('dahai:')
      ? botTurn.action.slice('dahai:'.length)
      : null

  return (
    <div className="space-y-1.5">
      <SourceBadge source={botTurn?.source ?? null} />
      {maintain.map((c, i) => {
        const botPick = picks?.find((p) => p.tile === c.discard) ?? null
        const isBotChosen = botPick?.chosen === true || forcedTile === c.discard
        return (
          <CandidateRow
            key={c.discard}
            cand={c}
            risk={risks[i]}
            winLive={winLive}
            riskLive={riskLive}
            isActual={actual !== null && c.discard === actual}
            isBest={best !== null && c.discard === best}
            botProb={botPick?.prob ?? null}
            isBotTop={picks != null && picks.length > 0 && picks[0].tile === c.discard}
            isBotChosen={isBotChosen}
          />
        )
      })}
    </div>
  )
}

/** Turn-level provenance of the bot's answer — the whole point is that
 *  a rule fast path carries no probability distribution to trust. */
function SourceBadge({ source }: { source: BotRefSource | null }) {
  const { t } = useTranslation()
  if (source == null || source === 'no_answer') return null
  const style =
    source === 'model'
      ? 'border-border text-muted-foreground'
      : source === 'rule_forced'
        ? 'border-amber-500/40 text-amber-600 dark:text-amber-400'
        : 'border-red-500/40 text-red-600 dark:text-red-400'
  return (
    <div className="flex justify-end">
      <span className={'rounded border px-1.5 py-0.5 text-[10px] ' + style}>
        {t(`tutorial.bot_source_${source}`)}
      </span>
    </div>
  )
}

function CandidateRow({
  cand,
  risk,
  winLive,
  riskLive,
  isActual,
  isBest,
  botProb,
  isBotTop,
  isBotChosen,
}: {
  cand: DiscardCandidate
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
              (isBotChosen ? 'font-semibold text-violet-600 dark:text-violet-400' : 'text-muted-foreground')
            }
          >
            {t('tutorial.bot_prob', { p: Math.round(botProb * 100) })}
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
