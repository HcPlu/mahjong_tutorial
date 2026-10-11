import { useEffect } from 'react'
import { HAS_TAURI, invoke, listen } from '@/lib/tauri'
import type {
  AnalysisResult,
  BotResponse,
  BotStatus,
  CaptureStatus,
  GameRecord,
  GameStateSnapshot,
  HistoryEvent,
  MahgenView,
  MjaiEvent,
  Notification,
  OverlayConfig,
  Snapshot,
} from '@/types'
import { useGameStore } from '@/stores/gameStore'
import { useAnalysisStore } from '@/stores/analysisStore'
import { useBotStore } from '@/stores/botStore'
import { useCaptureStore } from '@/stores/captureStore'
import { useNotifyStore } from '@/stores/notifyStore'
import { useApiStatusStore } from '@/stores/apiStatusStore'
import { useInstallStore } from '@/stores/installStore'
import { useConfigStore } from '@/stores/configStore'
import { useHistoryStore } from '@/stores/historyStore'
import { useTutorialStore } from '@/stores/tutorialStore'
import { toast, type ToastSeverity } from '@/components/ui/sonner'
import { evaluate } from '@/components/tutorial/commentary/engine'
import { CommentaryThrottle, turnKey } from '@/components/tutorial/commentary/throttle'
import type { TutorialLiveEvent } from '@/components/tutorial/types'
import { pickShow } from '@/lib/botShow'
import { getTutorialConfig } from '@/components/tutorial/configGate'

// Backend `Notification.level` ∈ {info,success,warn,error}; toast helper
// uses `warning`. Map across.
// Module-level: the per-turn throttle outlives any single bridge mount.
const coachThrottle = new CommentaryThrottle()

// When the current draw's auto explanation was queued (bot response time) —
// the draw→arrival delta is the latency the LLM log tile shows.
let llmDrawAt: number | null = null

const TOAST_SEVERITY: Record<Notification['level'], ToastSeverity> = {
  info: 'info',
  success: 'success',
  warn: 'warning',
  error: 'error',
}

// One-shot bridge mounted from <App>. Subscribes to all Tauri events,
// hydrates initial state, and unsubscribes on unmount.
export function useTauriBridge() {
  useEffect(() => {
    if (!HAS_TAURI) return

    const unlistens: Array<() => void> = []
    let cancelled = false

    const refreshGame = async () => {
      try {
        const [snap, view] = await Promise.all([
          invoke<GameStateSnapshot | null>('get_game_snapshot'),
          invoke<MahgenView | null>('get_mahgen_view'),
        ])
        if (cancelled) return
        useGameStore.getState().setGame(snap)
        useGameStore.getState().setView(view)
      } catch {
        /* ignore: backend may not be ready */
      }
    }

    ;(async () => {
      try {
        const status = await invoke<Snapshot>('get_status')
        if (cancelled) return
        useConfigStore.getState().setConfig(status.config)
        useConfigStore.getState().setLogDir(status.log_dir)
        useBotStore.getState().setStatus(status.bot_status)
        useCaptureStore.getState().set(status.capture_status)
      } catch {
        /* ignore */
      }
      await refreshGame()
      try {
        const a = await invoke<AnalysisResult | null>('get_analysis')
        if (!cancelled) useAnalysisStore.getState().set(a)
      } catch {
        /* ignore */
      }
      // Game history: load all records once at startup. The History
      // tab is the only consumer; loading lazily there forces a
      // round-trip on first nav, so do it eagerly while the bridge
      // is warm. Explicit large `limit` defends against any backend
      // version that still applies a default cap on `limit=0`.
      try {
        useHistoryStore.getState().setLoading(true)
        const records = await invoke<GameRecord[]>('list_game_history', {
          filter: null,
          limit: 1_000_000,
          offset: 0,
        })
        if (!cancelled) useHistoryStore.getState().setRecords(records)
      } catch {
        /* ignore: store stays empty */
      } finally {
        useHistoryStore.getState().setLoading(false)
      }
    })()

    listen<MjaiEvent>('mjai-event', (e) => {
      // Fresh game: clear any lingering online-API outage from the last one.
      if (e.type === 'start_game') {
        useApiStatusStore.getState().reset()
        coachThrottle.reset()
        useTutorialStore.getState().coachReset()
      }
      // Fresh hand: the previous hand's auto explanation is stale history
      // (junme and the cache key restart per hand on the backend).
      if (e.type === 'start_kyoku') {
        llmDrawAt = null
        useTutorialStore.getState().coachNewHand()
      }
      useNotifyStore.getState().pushEvent(e)
      void refreshGame()
    }).then((u) => unlistens.push(u))

    listen<AnalysisResult>('analysis-result', (a) => {
      useAnalysisStore.getState().set(a)
    }).then((u) => unlistens.push(u))

    listen<BotStatus>('bot-status', (s) => {
      useBotStore.getState().setStatus(s)
    }).then((u) => unlistens.push(u))

    listen<CaptureStatus>('capture-status', (s) => {
      useCaptureStore.getState().set(s)
    }).then((u) => unlistens.push(u))

    listen<BotResponse>('bot-response', (r) => {
      useNotifyStore.getState().pushResponse(r)
      // Coach template layer: the bot answering marks a decision point —
      // evaluate the rules once per turn against the latest local data
      // (numbers straight from the analysis store / bot meta, never new
      // math; D-0005).
      const cfg = useConfigStore.getState().config
      const tcfg = getTutorialConfig(cfg)
      if (tcfg.coach.enabled) {
        // Auto mode (D-0014): the backend queued a background generation
        // for this draw — show the pending state until the result event.
        if (tcfg.coach.auto_policy === 'all') {
          llmDrawAt = Date.now()
          useTutorialStore.getState().coachAutoTurnStart()
        }
        const snap = useGameStore.getState().game
        if (snap && coachThrottle.allow(turnKey(snap))) {
          const line = evaluate(
            useAnalysisStore.getState().result,
            pickShow(r.meta),
            snap,
          )
          if (line) useTutorialStore.getState().setCoachLine(line)
        }
      }
    }).then((u) => unlistens.push(u))

    // Auto-generated live deep-dive arrived (D-0014) — fresh generation
    // or cache replay; either way it belongs to the current draw.
    listen<TutorialLiveEvent>('tutorial-live-turn', (ev) => {
      const latencyMs = llmDrawAt == null ? null : Date.now() - llmDrawAt
      llmDrawAt = null
      useTutorialStore.getState().setAutoDoc(ev.doc)
      useTutorialStore.getState().appendLlmLog({
        ts: Date.now(),
        junme: ev.doc.junme,
        model: ev.model,
        latencyMs,
        source: 'auto',
        ok: true,
        headline: ev.doc.headline,
      })
    }).then((u) => unlistens.push(u))

    // The overlay window can turn itself off (its × button). Mirror that back
    // into the config store so the Game page's toggle doesn't keep claiming the
    // overlay is open.
    listen<OverlayConfig>('overlay-config', (o) => {
      useConfigStore.getState().setOverlay(o)
    }).then((u) => unlistens.push(u))

    listen<HistoryEvent>('history-recorded', (ev) => {
      if (ev.kind === 'recorded') {
        useHistoryStore.getState().prepend(ev.record)
      } else if (ev.kind === 'deleted') {
        useHistoryStore.getState().remove(ev.id)
      }
    }).then((u) => unlistens.push(u))

    listen<Notification>('notify', (n) => {
      useNotifyStore.getState().pushToast(n)
      // Drive the persistent "Online API" health indicator (Statusbar) off the
      // same channel the backend uses for degrade/recover toasts.
      if (n.id === 'native-api-health') {
        useApiStatusStore
          .getState()
          .setDegraded(n.level === 'warn' || n.level === 'error', n.body)
      }
      // Feed env install/sync progress to the blocking overlay (which is
      // shown for the duration of `withInstallBlock`). Ids: `bot-install-*`
      // (GitHub install/reinstall) and `bot-sync-*` ("Reinstall environment").
      if (n.id && (n.id.startsWith('bot-install-') || n.id.startsWith('bot-sync-'))) {
        useInstallStore.getState().setProgress(n)
      }
      toast[TOAST_SEVERITY[n.level]](n.title, {
        description: n.body,
        id: n.id,
        ...(n.sticky ? { duration: Infinity } : {}),
      })
    }).then((u) => unlistens.push(u))

    return () => {
      cancelled = true
      unlistens.forEach((u) => u())
    }
  }, [])
}
