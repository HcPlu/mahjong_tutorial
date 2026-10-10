// Discard-candidate comparison for the replay page: the engine's top
// maintain candidates with win-rate / deal-in-risk bars, plus badges for
// the student's actual pick and the engine's attack-best. Every number
// is taken verbatim from the local AnalysisResult — this component does
// no math beyond the engine's percent → 0..1 display conversion.

import { useTranslation } from 'react-i18next'

import { Mahgen } from '@/components/Mahgen'
import { mjaiToMahgen, tileIdx } from '@/lib/tileIdx'
import type { AnalysisResult, DiscardCandidate } from '@/types'

type Props = {
  analysis: AnalysisResult
  /** The student's actual discard this turn (mjai tile), when known. */
  actual: string | null
}

const MAX_ROWS = 5

export function CandidateChart({ analysis, actual }: Props) {
  const maintain = analysis.hand14?.maintain?.slice(0, MAX_ROWS) ?? []
  if (maintain.length === 0) return null
  const best = analysis.best_attack_discard

  return (
    <div className="space-y-1.5">
      {maintain.map((c) => (
        <CandidateRow
          key={c.discard}
          cand={c}
          risk={riskFor(analysis, c.discard)}
          isActual={actual !== null && c.discard === actual}
          isBest={best !== null && c.discard === best}
        />
      ))}
    </div>
  )
}

function CandidateRow({
  cand,
  risk,
  isActual,
  isBest,
}: {
  cand: DiscardCandidate
  /** Deal-in risk for the discarded tile, 0..1 (mixed risk / 100). */
  risk: number | null
  isActual: boolean
  isBest: boolean
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
        </div>
        <div className="flex items-center gap-2">
          <Bar value={win} className="bg-emerald-500/70" />
          <span className="w-12 shrink-0 text-right font-mono text-xs tabular-nums text-emerald-600 dark:text-emerald-400">
            {(win * 100).toFixed(0)}%
          </span>
          <Bar value={risk ?? 0} className="bg-red-500/70" />
          <span className="w-12 shrink-0 text-right font-mono text-xs tabular-nums text-red-600 dark:text-red-400">
            {risk === null ? '—' : `${(risk * 100).toFixed(1)}%`}
          </span>
        </div>
      </div>
      <div className="flex shrink-0 flex-col items-end gap-1">
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
      </div>
    </div>
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
