// In-game coach tile (M2, D-0008): the template layer's deterministic
// one-liner plus the on-demand LLM deep dive. Rendering is gated by
// `[tutorial.coach] enabled`; the deep-dive button additionally needs
// the outbound gate (consent + configured endpoint) — otherwise it
// explains where to fix that instead.

import { useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { Loader2, Sparkles, Volume2, VolumeX } from 'lucide-react'
import { toast } from 'sonner'

import { Mahgen } from '@/components/Mahgen'
import { TileFrame } from '@/components/TileFrame'
import { Button } from '@/components/ui/button'
import { mjaiToMahgen } from '@/lib/tileIdx'
import { getTutorialConfig, outboundAllowed } from '@/components/tutorial/configGate'
import { useConfigStore } from '@/stores/configStore'
import { useTutorialStore } from '@/stores/tutorialStore'
import type { Breakpoint } from '@/tiles/defaults'

export function CommentaryTile({ bp }: { bp: Breakpoint }) {
  const { t } = useTranslation()
  const config = useConfigStore((s) => s.config)
  const coachCfg = getTutorialConfig(config).coach
  const line = useTutorialStore((s) => s.coach.line)
  const muted = useTutorialStore((s) => s.coach.muted)
  const deepDoc = useTutorialStore((s) => s.coach.deepDoc)
  const deepLoading = useTutorialStore((s) => s.coach.deepLoading)
  const listRef = useRef<HTMLDivElement>(null)

  const cfg = getTutorialConfig(config)
  const canOutbound = outboundAllowed(cfg)
  const showDeep = coachCfg.deep_dive_button

  const lineNode = () => {
    if (!coachCfg.enabled) {
      return <p className="text-sm text-muted-foreground">{t('coach.disabled')}</p>
    }
    if (muted) {
      return <p className="text-sm text-muted-foreground">{t('coach.muted')}</p>
    }
    if (!line) {
      return <p className="text-sm text-muted-foreground">{t('coach.waiting')}</p>
    }
    return (
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm leading-relaxed">
          {t(line.i18nKey, line.params as Record<string, string | number>)}
        </span>
        {line.tiles.length > 0 && (
          <Mahgen seq={mjaiToMahgen(line.tiles)} kind="bot-show" containerRef={listRef} />
        )}
      </div>
    )
  }

  return (
    <TileFrame
      id="coach"
      title={t('tile.commentary')}
      bp={bp}
      rightSlot={
        coachCfg.enabled ? (
          <Button
            size="icon"
            variant="ghost"
            className="h-6 w-6"
            onClick={() => useTutorialStore.getState().coachToggleMute()}
            aria-label={t('coach.mute_toggle')}
          >
            {muted ? (
              <VolumeX className="h-3.5 w-3.5" />
            ) : (
              <Volume2 className="h-3.5 w-3.5" />
            )}
          </Button>
        ) : undefined
      }
    >
      <div ref={listRef} className="flex h-full flex-col gap-2">
        {lineNode()}
        {showDeep && (
          <div className="mt-auto space-y-2">
            <Button
              size="sm"
              variant={canOutbound ? 'outline' : 'ghost'}
              disabled={deepLoading || !canOutbound}
              title={canOutbound ? undefined : t('coach.deep_needs_config')}
              onClick={() => void runDeep()}
            >
              {deepLoading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Sparkles className="h-3.5 w-3.5" />
              )}
              {deepLoading ? t('coach.deep_loading') : t('coach.deep_dive')}
            </Button>
            {deepDoc && (
              <div className="rounded-md border border-border px-2 py-1.5">
                {deepDoc.headline && (
                  <p className="text-sm font-semibold">{deepDoc.headline}</p>
                )}
                <p className="text-sm leading-relaxed">{deepDoc.explanation_zh}</p>
              </div>
            )}
          </div>
        )}
      </div>
    </TileFrame>
  )
}

async function runDeep() {
  const store = useTutorialStore.getState()
  await store.explainLive()
  const err = useTutorialStore.getState().coach.deepError
  if (err) toast.error(err)
}
