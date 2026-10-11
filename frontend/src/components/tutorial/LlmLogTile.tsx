// In-game LLM activity log tile (fork, D-0018): every live-turn
// generation as it lands — time, turn, source, draw→arrival latency,
// model, headline (or the error). The UI-side counterpart of the
// `live turn N explained in X.Xs` lines the backend writes to all.log.

import { useTranslation } from 'react-i18next'

import { TileFrame } from '@/components/TileFrame'
import { tileNamesZh } from '@/components/tutorial/tileText'
import { useTutorialStore } from '@/stores/tutorialStore'
import type { Breakpoint } from '@/tiles/defaults'

function fmtTime(ts: number): string {
  return new Date(ts).toLocaleTimeString([], { hour12: false })
}

export function LlmLogTile({ bp }: { bp: Breakpoint }) {
  const { t } = useTranslation()
  const log = useTutorialStore((s) => s.llmLog)
  // Newest first — the interesting event is always the last one.
  const entries = [...log].reverse()

  return (
    <TileFrame id="llm-log" title={t('tile.llm_log')} bp={bp}>
      {entries.length === 0 ? (
        <p className="p-2 text-sm text-muted-foreground">{t('tutorial.llm_log_empty')}</p>
      ) : (
        <div className="h-full overflow-y-auto px-2 py-1 text-xs leading-relaxed">
          {entries.map((e) => (
            <div key={e.id} className="border-b border-border/50 py-1 last:border-0">
              <div className="flex flex-wrap items-center gap-x-2 text-muted-foreground">
                <span className="font-mono">{fmtTime(e.ts)}</span>
                {e.junme != null && (
                  <span>{t('overlay.explain_turn', { junme: e.junme })}</span>
                )}
                <span>{e.source === 'auto' ? t('tutorial.llm_log_auto') : t('tutorial.llm_log_manual')}</span>
                {e.latencyMs != null && <span>{(e.latencyMs / 1000).toFixed(1)}s</span>}
                {e.model && <span className="truncate">{e.model}</span>}
              </div>
              {e.ok ? (
                e.headline && (
                  <p className="truncate text-foreground">{tileNamesZh(e.headline)}</p>
                )
              ) : (
                <p className="truncate text-destructive">{e.error}</p>
              )}
            </div>
          ))}
        </div>
      )}
    </TileFrame>
  )
}
