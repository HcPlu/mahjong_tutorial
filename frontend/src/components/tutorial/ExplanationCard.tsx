// LLM explanation card for one replay decision: headline + coaching text
// + the recommended-vs-actual tiles + the engine numbers the text is
// allowed to quote (already cross-checked by the backend's download
// zone — this component just renders).
//
// The coaching text toggles between two views: tile codes translated to
// Chinese tile names (default — 一筒 instead of 1p) and the verbatim
// LLM output. Translation is display-only post-processing; the stored
// document never changes.

import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Languages, FileText } from 'lucide-react'

import { Mahgen } from '@/components/Mahgen'
import { Button } from '@/components/ui/button'
import { mjaiToMahgen } from '@/lib/tileIdx'
import { tileNamesZh } from '@/components/tutorial/tileText'
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
  const [raw, setRaw] = useState(false)
  const recommended = labelToTile(doc.recommended)
  const view = (s: string) => (raw ? s : tileNamesZh(s))

  return (
    <div className="space-y-2">
      <div className="flex items-start justify-between gap-2">
        {doc.headline ? (
          <p className="text-sm font-semibold">{view(doc.headline)}</p>
        ) : (
          <span />
        )}
        <Button
          variant="ghost"
          size="sm"
          className="h-6 shrink-0 px-2 text-xs text-muted-foreground"
          onClick={() => setRaw((v) => !v)}
          title={raw ? t('tutorial.show_translated') : t('tutorial.show_raw')}
        >
          {raw ? <Languages className="h-3 w-3" /> : <FileText className="h-3 w-3" />}
          {raw ? t('tutorial.show_translated') : t('tutorial.show_raw')}
        </Button>
      </div>
      <p className="text-sm leading-relaxed text-foreground/90">
        {view(doc.explanation_zh)}
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
