// Tutorial (AI coaching) Zustand store: the replay stepper state, the
// LLM review document, and the coverage preference. The coach slice
// (in-game commentary) lives here too — see components/tutorial.
//
// Data flow: `openGame` pulls the replay outline + any cached review;
// stepping pulls per-event snapshots (`tutorial_replay_step`) lazily.
// Generation goes through `tutorial_generate_review` (cache-first on
// the backend; the store just reflects the outcome).

import { create } from 'zustand'

import { invoke } from '@/lib/tauri'
import type { CommentaryLine } from '@/components/tutorial/commentary/engine'
import type {
  BotRefDoc,
  ReplayOutline,
  ReplayStep,
  ReviewCoverage,
  ReviewDoc,
  TurnDoc,
} from '@/components/tutorial/types'

const COVERAGE_KEY = 'akagi.tutorial.coverage'

/** One recorded in-game LLM interaction (auto deep-dive or manual click) —
 *  the tile-side view of what all.log records on the backend. */
export type LlmLogEntry = {
  id: number
  /** Arrival time (`Date.now()`). */
  ts: number
  /** Turn the explanation belongs to. */
  junme: number | null
  model: string | null
  /** Draw → arrival latency when observable (auto mode); null otherwise. */
  latencyMs: number | null
  source: 'auto' | 'manual'
  ok: boolean
  headline?: string | null
  error?: string
}

/** Newest entries worth keeping in the tile without it becoming a history browser. */
const LLM_LOG_CAP = 50
let llmLogSeq = 0

function loadCoverage(): ReviewCoverage {
  if (typeof localStorage === 'undefined') return 'key'
  return localStorage.getItem(COVERAGE_KEY) === 'all' ? 'all' : 'key'
}

function saveCoverage(c: ReviewCoverage) {
  try {
    localStorage.setItem(COVERAGE_KEY, c)
  } catch {
    /* quota — ignore */
  }
}

type TutorialStore = {
  gameId: string | null
  outline: ReplayOutline | null
  doc: ReviewDoc | null
  generating: boolean
  generateError: string | null
  /** Current replay position (event index into the stream). */
  stepIndex: number
  step: ReplayStep | null
  coverage: ReviewCoverage
  /** Offline bot reference ("what would the bot have done"), per game. */
  botRef: BotRefDoc | null
  botRefLoading: boolean
  botRefError: string | null
  /** Bot answers recorded while the game was being played (D-0017) —
   *  the bot that actually drove the game, which may differ from the
   *  regenerated `botRef`'s bot. Read-only cache; never regenerated. */
  liveBot: BotRefDoc | null

  /** Load outline + cached review for a game and jump to its first
   *  decision point (fall back to event 0 when there are none). */
  openGame: (gameId: string) => Promise<void>
  reset: () => void
  setStepIndex: (index: number) => Promise<void>
  stepBy: (delta: number) => Promise<void>
  jumpToDecision: (junme: number) => Promise<void>
  setCoverage: (coverage: ReviewCoverage) => void
  /** Generate (or re-generate) the LLM review for the open game. */
  generate: () => Promise<void>
  /** Load (replaying the game through the bot if uncached) the bot
   *  reference for the open game. `silent` (the auto-run on open) drops
   *  errors instead of showing them: configs without a replayable bot
   *  would otherwise greet every replay with an undismissable error —
   *  the manual button still surfaces them. */
  loadBotRef: (silent?: boolean) => Promise<void>

  // --- in-game coach slice (M2, D-0008; auto mode D-0014) ---
  coach: {
    /** The current template-layer line (already throttled upstream). */
    line: CommentaryLine | null
    /** User muted the coach for this game (reset on start_game). */
    muted: boolean
    /** Latest deep-dive explanation (cached per turn on the backend). */
    deepDoc: TurnDoc | null
    deepLoading: boolean
    deepError: string | null
    /** Auto-generated explanation for the current draw (D-0014): set on
     *  `tutorial-live-turn`, cleared on the next bot response. Fresh for
     *  exactly one turn, so it wins over the possibly-stale deepDoc. */
    autoDoc: TurnDoc | null
    /** An auto explanation is in flight for the current draw. */
    autoPending: boolean
  }
  setCoachLine: (line: CommentaryLine | null) => void
  coachToggleMute: () => void
  /** New game: clear the line, the mute and the deep dive. */
  coachReset: () => void
  explainLive: () => Promise<void>
  /** New draw while auto mode is on: retire the previous turn's doc and
   *  mark the new one as pending. */
  coachAutoTurnStart: () => void
  /** New hand: the previous hand's auto explanation is history (the line,
   *  mute and manual deep dive survive). */
  coachNewHand: () => void
  /** `tutorial-live-turn` arrived: publish it. */
  setAutoDoc: (doc: TurnDoc) => void

  // --- in-game LLM activity log (D-0018) ---
  llmLog: LlmLogEntry[]
  appendLlmLog: (entry: Omit<LlmLogEntry, 'id'>) => void
}

async function fetchStep(gameId: string, index: number): Promise<ReplayStep | null> {
  return invoke<ReplayStep | null>('tutorial_replay_step', {
    gameId,
    eventIndex: index,
  })
}

export const useTutorialStore = create<TutorialStore>((set, get) => ({
  gameId: null,
  outline: null,
  doc: null,
  generating: false,
  generateError: null,
  stepIndex: 0,
  step: null,
  coverage: loadCoverage(),
  botRef: null,
  botRefLoading: false,
  botRefError: null,
  liveBot: null,

  coach: {
    line: null,
    muted: false,
    deepDoc: null,
    deepLoading: false,
    deepError: null,
    autoDoc: null,
    autoPending: false,
  },

  llmLog: [],

  setCoachLine: (line) => set((s) => ({ coach: { ...s.coach, line } })),
  coachToggleMute: () => set((s) => ({ coach: { ...s.coach, muted: !s.coach.muted } })),
  coachReset: () =>
    set({
      coach: {
        line: null,
        muted: false,
        deepDoc: null,
        deepLoading: false,
        deepError: null,
        autoDoc: null,
        autoPending: false,
      },
    }),

  coachAutoTurnStart: () =>
    set((s) => ({ coach: { ...s.coach, autoDoc: null, autoPending: true } })),

  coachNewHand: () =>
    set((s) => ({ coach: { ...s.coach, autoDoc: null, autoPending: false } })),

  setAutoDoc: (doc) =>
    set((s) => ({ coach: { ...s.coach, autoDoc: doc, autoPending: false } })),

  appendLlmLog: (entry) =>
    set((s) => ({
      llmLog: [...s.llmLog, { ...entry, id: ++llmLogSeq }].slice(-LLM_LOG_CAP),
    })),

  explainLive: async () => {
    if (get().coach.deepLoading) return
    set((s) => ({ coach: { ...s.coach, deepLoading: true, deepError: null } }))
    try {
      const deepDoc = await invoke<TurnDoc>('tutorial_explain_live')
      set((s) => ({ coach: { ...s.coach, deepDoc, deepLoading: false } }))
      get().appendLlmLog({
        ts: Date.now(),
        junme: deepDoc.junme,
        model: null,
        latencyMs: null,
        source: 'manual',
        ok: true,
        headline: deepDoc.headline,
      })
    } catch (e) {
      const error = String(e)
      set((s) => ({
        coach: { ...s.coach, deepLoading: false, deepError: error },
      }))
      get().appendLlmLog({
        ts: Date.now(),
        junme: null,
        model: null,
        latencyMs: null,
        source: 'manual',
        ok: false,
        error,
      })
    }
  },

  openGame: async (gameId) => {
    set({
      gameId,
      outline: null,
      doc: null,
      generating: false,
      generateError: null,
      step: null,
      stepIndex: 0,
      botRef: null,
      botRefLoading: false,
      botRefError: null,
      liveBot: null,
    })
    try {
      const [outline, doc, botRef, liveBot] = await Promise.all([
        invoke<ReplayOutline | null>('tutorial_replay_outline', { gameId }),
        invoke<ReviewDoc | null>('tutorial_get_review', { gameId }),
        invoke<BotRefDoc | null>('tutorial_get_bot_reference', { gameId }),
        invoke<BotRefDoc | null>('tutorial_get_live_bot', { gameId }),
      ])
      const firstDecision = outline?.decisions[0]?.event_index ?? 0
      set({ outline, doc, botRef, liveBot })
      const step = await fetchStep(gameId, firstDecision)
      set({ stepIndex: firstDecision, step })
    } catch (e) {
      set({ generateError: String(e) })
    }
  },

  reset: () =>
    set({
      gameId: null,
      outline: null,
      doc: null,
      generating: false,
      generateError: null,
      step: null,
      stepIndex: 0,
      botRef: null,
      botRefLoading: false,
      botRefError: null,
      liveBot: null,
    }),

  setStepIndex: async (index) => {
    const { gameId, outline } = get()
    if (!gameId || !outline) return
    const clamped = Math.max(0, Math.min(outline.total_events - 1, index))
    set({ stepIndex: clamped })
    try {
      const step = await fetchStep(gameId, clamped)
      // Drop stale responses if the user stepped on meanwhile.
      if (get().stepIndex === clamped) set({ step })
    } catch {
      /* transient — keep the previous step */
    }
  },

  stepBy: async (delta) => {
    await get().setStepIndex(get().stepIndex + delta)
  },

  jumpToDecision: async (junme) => {
    const d = get().outline?.decisions.find((x) => x.junme === junme)
    if (d) await get().setStepIndex(d.event_index)
  },

  setCoverage: (coverage) => {
    saveCoverage(coverage)
    set({ coverage })
  },

  loadBotRef: async (silent = false) => {
    const { gameId, botRefLoading } = get()
    if (!gameId || botRefLoading) return
    set({ botRefLoading: true, botRefError: null })
    try {
      const botRef = await invoke<BotRefDoc>('tutorial_bot_reference', { gameId })
      set({ botRef, botRefLoading: false })
    } catch (e) {
      set({
        botRefLoading: false,
        botRefError: silent ? null : String(e),
      })
    }
  },

  generate: async () => {
    const { gameId, coverage, generating } = get()
    if (!gameId || generating) return
    set({ generating: true, generateError: null })
    try {
      // Regeneration: drop the cache first so the backend actually calls
      // the LLM again (it is cache-first by design).
      if (get().doc) {
        await invoke<boolean>('tutorial_delete_review', { gameId })
      }
      const doc = await invoke<ReviewDoc>('tutorial_generate_review', {
        gameId,
        coverage,
      })
      set({ doc, generating: false })
    } catch (e) {
      set({ generating: false, generateError: String(e) })
    }
  },
}))
