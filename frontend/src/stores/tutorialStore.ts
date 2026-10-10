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
import type {
  ReplayOutline,
  ReplayStep,
  ReviewCoverage,
  ReviewDoc,
} from '@/components/tutorial/types'

const COVERAGE_KEY = 'akagi.tutorial.coverage'

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

  openGame: async (gameId) => {
    set({
      gameId,
      outline: null,
      doc: null,
      generating: false,
      generateError: null,
      step: null,
      stepIndex: 0,
    })
    try {
      const [outline, doc] = await Promise.all([
        invoke<ReplayOutline | null>('tutorial_replay_outline', { gameId }),
        invoke<ReviewDoc | null>('tutorial_get_review', { gameId }),
      ])
      const firstDecision = outline?.decisions[0]?.event_index ?? 0
      set({ outline, doc })
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
