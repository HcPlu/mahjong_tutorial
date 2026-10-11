import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { X } from 'lucide-react'
import { BotShowList } from '@/components/BotShowList'
import { getTutorialConfig, outboundAllowed } from '@/components/tutorial/configGate'
import { tileNamesZh } from '@/components/tutorial/tileText'
import type { TutorialLiveEvent, TurnDoc } from '@/components/tutorial/types'
import { pickShow, visibleItems } from '@/lib/botShow'
import { invoke, listen } from '@/lib/tauri'
import type { AppConfig, BotResponse, OverlayConfig, ShowMeta } from '@/types'

// The always-on-top overlay window's entire UI. Mounted by main.tsx instead of
// the router when the window label is `overlay` — no sidebar, no statusbar, no
// route tree, and deliberately no `useTauriBridge`: this window wants one event
// (`bot-response`), not the whole app's state.
//
// The window itself is transparent; the rounded card below is the only thing
// that gets drawn. Everything inside the card is `pointer-events: none` so that
// a mousedown anywhere lands on the drag region and moves the window — except
// the close button, which opts back in.

// Only used if `get_config` fails — a blank overlay would be worse than a
// default-looking one. Mirrors `OverlayConfig::default()`.
const FALLBACK: OverlayConfig = {
  enabled: true,
  top_n: 3,
  opacity: 0.95,
  always_on_top: true,
}

export function Overlay() {
  const { t } = useTranslation()
  const [show, setShow] = useState<ShowMeta | null>(null)
  const [cfg, setCfg] = useState<OverlayConfig>(FALLBACK)
  const [auto, setAuto] = useState(false)
  const [coachAuto, setCoachAuto] = useState(false)
  const [canOutbound, setCanOutbound] = useState(false)
  const [autoDoc, setAutoDoc] = useState<TurnDoc | null>(null)
  const [autoPending, setAutoPending] = useState(false)

  useEffect(() => {
    const unlistens: Array<() => void> = []
    let cancelled = false

    const refreshConfig = (c: AppConfig) => {
      setCfg(c.overlay)
      setAuto(c.autoplay?.enabled ?? false)
      const tcfg = getTutorialConfig(c)
      setCoachAuto(tcfg.coach.auto_policy === 'all')
      setCanOutbound(outboundAllowed(tcfg))
    }

    // The window is opened by the backend, so config is already on disk;
    // `overlay-config` keeps us in step with later edits in Settings — every
    // Settings save broadcasts it, so autoplay flips made there also reach
    // this toggle.
    invoke<AppConfig>('get_config')
      .then((c) => {
        if (!cancelled) refreshConfig(c)
      })
      .catch(() => {
        /* keep the fallback — a blank overlay would be worse than a default one */
      })

    listen<BotResponse>('bot-response', (r) => {
      const s = pickShow(r.meta)
      // Responses without a `show` block (e.g. a bare `none`) must not wipe
      // the last real suggestion off the screen.
      if (s) setShow(s)
      // A bot reply marks a fresh decision point: with auto explanations
      // on, this draw's generation is now in flight.
      setAutoPending(true)
    }).then((u) => unlistens.push(u))

    // Auto-generated live explanation arrived (fork): rendered under the
    // suggestions. Labeled with its junme so one still waiting for the
    // next decision reads as clearly belonging to that turn.
    listen<TutorialLiveEvent>('tutorial-live-turn', (ev) => {
      setAutoDoc(ev.doc)
      setAutoPending(false)
    }).then((u) => unlistens.push(u))

    listen<OverlayConfig>('overlay-config', () => {
      // Refetch the whole config: the event payload only carries the overlay
      // section, while this window also mirrors `autoplay.enabled`.
      invoke<AppConfig>('get_config')
        .then((c) => {
          if (!cancelled) refreshConfig(c)
        })
        .catch(() => {})
    }).then((u) => unlistens.push(u))

    return () => {
      cancelled = true
      unlistens.forEach((u) => u())
    }
  }, [])

  const items = visibleItems(show, cfg.top_n)

  // Closing the overlay persists `enabled = false`, so it stays closed across
  // restarts. Settings is the way back.
  const close = () => {
    void invoke('set_overlay_enabled', { enabled: false }).catch(() => {})
  }

  // In-game autoplay toggle: applies immediately (the backend flips the
  // shared config, and the manager re-reads it every decision), optimistic
  // with a convergence on failure: refetch what the backend actually has
  // instead of blindly flipping back — a blind revert can disagree with
  // the real setting (e.g. the write raced another window's save), and a
  // button that lies about its state invites a wrong-direction click.
  const toggleAuto = () => {
    const next = !auto
    setAuto(next)
    invoke('set_autoplay_enabled', { enabled: next }).catch(() => {
      invoke<AppConfig>('get_config')
        .then((c) => setAuto(c.autoplay?.enabled ?? false))
        .catch(() => setAuto(!next))
    })
  }

  // Explain toggle: one click = coach on + one explanation per decision
  // (`set_coach_auto`). Same optimistic-with-convergence shape as the
  // autoplay toggle. Enabling also clears the pending flag — the backend
  // only queues on the *next* bot reply, so nothing is in flight yet.
  const toggleExplain = () => {
    const next = !coachAuto
    setCoachAuto(next)
    if (next) setAutoPending(false)
    invoke('set_coach_auto', { enabled: next }).catch(() => {
      invoke<AppConfig>('get_config')
        .then((c) => {
          const tcfg = getTutorialConfig(c)
          setCoachAuto(tcfg.coach.auto_policy === 'all')
          setCanOutbound(outboundAllowed(tcfg))
        })
        .catch(() => setCoachAuto(!next))
    })
  }

  return (
    <div
      data-tauri-drag-region
      className="h-screen w-screen overflow-hidden p-1 select-none cursor-grab active:cursor-grabbing"
    >
      <div
        data-tauri-drag-region
        className="flex h-full w-full flex-col overflow-hidden rounded-lg border border-border bg-background shadow-lg [&_*]:pointer-events-none"
        style={{ opacity: cfg.opacity }}
      >
        <header
          data-tauri-drag-region
          className="flex shrink-0 items-center gap-1 px-2 pt-1.5 pb-1"
        >
          <span className="flex-1 truncate text-[11px] font-medium text-muted-foreground">
            {show?.title ?? t('tile.bot_show_default_title')}
          </span>
          <button
            type="button"
            onClick={toggleExplain}
            aria-pressed={coachAuto}
            aria-label={coachAuto ? t('overlay.explain_off_title') : t('overlay.explain_on_title')}
            title={coachAuto ? t('overlay.explain_off_title') : t('overlay.explain_on_title')}
            className={
              'shrink-0 cursor-pointer rounded border px-1.5 py-px text-[10px] font-semibold tracking-wide ' +
              (coachAuto
                ? 'border-violet-500/60 bg-violet-500/15 text-violet-500'
                : 'border-border text-muted-foreground hover:text-foreground')
            }
            style={{ pointerEvents: 'auto' }}
          >
            {t('overlay.explain_label')}
          </button>
          <button
            type="button"
            onClick={toggleAuto}
            aria-pressed={auto}
            aria-label={auto ? t('overlay.auto_off_title') : t('overlay.auto_on_title')}
            title={auto ? t('overlay.auto_off_title') : t('overlay.auto_on_title')}
            className={
              'shrink-0 cursor-pointer rounded border px-1.5 py-px text-[10px] font-semibold tracking-wide ' +
              (auto
                ? 'border-emerald-500/60 bg-emerald-500/15 text-emerald-500'
                : 'border-border text-muted-foreground hover:text-foreground')
            }
            style={{ pointerEvents: 'auto' }}
          >
            {t('overlay.auto_label')}
          </button>
          <button
            type="button"
            onClick={close}
            aria-label={t('overlay.close')}
            title={t('overlay.close')}
            className="shrink-0 cursor-pointer text-muted-foreground hover:text-foreground"
            style={{ pointerEvents: 'auto' }}
          >
            <X className="size-3.5" />
          </button>
        </header>

        {/* Auto-explanation panel (fork, D-0018): the latest LLM deep-dive
            for the current decision, ABOVE the suggestions — it is the
            teaching content, the list is the reference. shrink-0 so the
            list below gives up the height, not the window. */}
        {coachAuto && (
          <div
            className="shrink-0 border-b border-border px-2 pt-1 pb-1.5"
            data-testid="overlay-explain-panel"
          >
            {autoDoc ? (
              <div data-testid="overlay-explain-doc">
                <p className="text-[10px] font-medium text-muted-foreground">
                  {t('overlay.explain_turn', { junme: autoDoc.junme })}
                </p>
                {autoDoc.headline && (
                  <p className="text-[11px] font-semibold leading-snug">
                    {tileNamesZh(autoDoc.headline)}
                  </p>
                )}
                <p className="line-clamp-4 text-[11px] leading-snug text-muted-foreground">
                  {tileNamesZh(autoDoc.explanation_zh)}
                </p>
              </div>
            ) : (
              // No spinner while unconfigured — the backend silently skips
              // every turn then, and "generating…" would misread as slow.
              autoPending &&
              canOutbound && (
                <p
                  className="text-[11px] text-muted-foreground"
                  data-testid="overlay-explain-pending"
                >
                  {t('overlay.explain_pending')}
                </p>
              )
            )}
            {!canOutbound && (
              <p className="text-[10px] leading-snug text-muted-foreground">
                {t('overlay.explain_unconfigured')}
              </p>
            )}
          </div>
        )}

        {/* min-h-0 is what gives this a definite height to hand down: without it
            a flex child refuses to shrink below its content, the rows never get
            a height to split, and the list collapses to hugging its content. */}
        <div className="min-h-0 flex-1 overflow-hidden px-1.5 pt-1 pb-1.5">
          {items.length === 0 ? (
            <span className="px-1 text-xs text-muted-foreground">{t('overlay.empty')}</span>
          ) : (
            <BotShowList items={items} variant="overlay" />
          )}
        </div>
      </div>
    </div>
  )
}
