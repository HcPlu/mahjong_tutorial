// In-game coach tile (M2, D-0008; auto mode D-0014): the template
// layer's deterministic one-liner, the on-demand LLM deep dive, and —
// when `[tutorial.coach] auto_policy = "all"` — the auto-generated
// explanation that lands a few seconds after each draw (pending spinner
// first). Rendering is gated by `[tutorial.coach] enabled`; the deep-dive
// layers additionally need the outbound gate (consent + configured
// endpoint) — otherwise they explain where to fix that instead.

import { useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { Loader2, Sparkles, Volume2, VolumeX } from 'lucide-react'
import { toast } from 'sonner'

import { Mahgen } from '@/components/Mahgen'
import { TileFrame } from '@/components/TileFrame'
import { Button } from '@/components/ui/button'
import { mjaiToMahgen } from '@/lib/tileIdx'
import { getTutorialConfig, outboundAllowed } from '@/components/tutorial/configGate'
import { tileNamesZh } from '@/components/tutorial/tileText'
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
  const autoDoc = useTutorialStore((s) => s.coach.autoDoc)
  const autoPending = useTutorialStore((s) => s.coach.autoPending)
  const listRef = useRef<HTMLDivElement>(null)

  const cfg = getTutorialConfig(config)
  const canOutbound = outboundAllowed(cfg)
  const showDeep = coachCfg.deep_dive_button
  // Auto mode: only promise a pending spinner when the backend can
  // actually generate (enabled + policy + outbound gate), and stay quiet
  // while muted — the user asked for silence.
  const autoOn =
    coachCfg.enabled && coachCfg.auto_policy === 'all' && canOutbound && !muted
  // The auto doc is fresh for this exact draw; a lingering manual deepDoc
  // may describe an older turn, so the auto doc wins whenever present.
  const shownDoc = autoOn && autoDoc ? autoDoc : deepDoc

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
        {(showDeep || autoOn) && (
          <div className="mt-auto space-y-2">
            {autoOn && autoPending && !shownDoc && (
              <p
                className="flex items-center gap-1.5 text-xs text-muted-foreground"
                data-testid="coach-auto-pending"
              >
                <Loader2 className="h-3 w-3 animate-spin" />
                {t('coach.auto_generating')}
              </p>
            )}
            {autoOn && autoDoc && (
              <div
                className="rounded-md border border-border px-2 py-1.5"
                data-testid="coach-auto-doc"
              >
                {autoDoc.headline && (
                  <p className="text-sm font-semibold">{tileNamesZh(autoDoc.headline)}</p>
                )}
                <p className="text-sm leading-relaxed">
                  {tileNamesZh(autoDoc.explanation_zh)}
                </p>
              </div>
            )}
            {showDeep && (
              <>
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
                {deepDoc && !(autoOn && autoDoc) && (
                  <div className="rounded-md border border-border px-2 py-1.5">
                    {deepDoc.headline && (
                      <p className="text-sm font-semibold">
                        {tileNamesZh(deepDoc.headline)}
                      </p>
                    )}
                    <p className="text-sm leading-relaxed">
                      {tileNamesZh(deepDoc.explanation_zh)}
                    </p>
                  </div>
                )}
              </>
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
