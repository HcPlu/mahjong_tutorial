// LLM explanation card for one replay decision: headline + coaching text
// + the recommended-vs-actual tiles + the engine numbers the text is
// allowed to quote (already cross-checked by the backend's download
// zone — this component just renders).

import { useTranslation } from 'react-i18next'

import { Mahgen } from '@/components/Mahgen'
import { mjaiToMahgen } from '@/lib/tileIdx'
import type { TurnDoc } from '@/components/tutorial/types'

type Props = {
  doc: TurnDoc
  /** The student's actual discard, mjai tile — null on live turns. */
  actual: string | null
}

function labelToTile(label: string): string | null {
  const tile = label.startsWith('dahai:') ? label.slice('dahai:'.length) : null
  return tile && tile.length > 0 ? tile : null
}

export function ExplanationCard({ doc, actual }: Props) {
  const { t } = useTranslation()
  const recommended = labelToTile(doc.recommended)

  return (
    <div className="space-y-2">
      {doc.headline && (
        <p className="text-sm font-semibold">{doc.headline}</p>
      )}
      <p className="text-sm leading-relaxed text-foreground/90">
        {doc.explanation_zh}
      </p>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-md border border-border px-2 py-1.5">
        {recommended && (
          <div className="flex items-center gap-1.5">
            <span className="text-xs text-muted-foreground">
              {t('tutorial.recommended')}
            </span>
            <Mahgen seq={mjaiToMahgen([recommended])} kind="bot-show" />
          </div>
        )}
        {actual && (
          <div className="flex items-center gap-1.5">
            <span className="text-xs text-muted-foreground">
              {t('tutorial.actual_pick')}
            </span>
            <Mahgen seq={mjaiToMahgen([actual])} kind="bot-show" />
          </div>
        )}
        <div className="ml-auto flex items-center gap-3 font-mono text-xs tabular-nums text-muted-foreground">
          <span>
            {t('tutorial.shanten')} {doc.numbers.shanten}
          </span>
          {doc.numbers.win_rate !== null && (
            <span>
              {t('tutorial.win_rate')} {(doc.numbers.win_rate * 100).toFixed(0)}%
            </span>
          )}
          {doc.numbers.deal_in_rate !== null && (
            <span>
              {t('tutorial.deal_in_rate')}{' '}
              {(doc.numbers.deal_in_rate * 100).toFixed(1)}%
            </span>
          )}
          {doc.numbers.avg_score !== null && (
            <span>
              {t('tutorial.avg_score')} {doc.numbers.avg_score.toLocaleString()}
            </span>
          )}
        </div>
      </div>
    </div>
  )
}
