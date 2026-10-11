import { beforeEach, describe, expect, it, vi } from 'vitest'

// The store shells out to Tauri commands; a mutable mock table lets each
// test script the backend responses (and assert the exact commands).
const calls: { cmd: string; args: Record<string, unknown> }[] = []
const responses = new Map<string, unknown>()

vi.mock('@/lib/tauri', () => ({
  invoke: vi.fn(async (cmd: string, args: Record<string, unknown>) => {
    calls.push({ cmd, args })
    if (responses.has(cmd)) return responses.get(cmd)
    throw new Error(`unexpected command: ${cmd}`)
  }),
}))

async function freshStore() {
  vi.resetModules()
  const mod = await import('./tutorialStore')
  return mod
}

const outline = {
  seat: 0,
  num_players: 4,
  total_events: 20,
  decisions: [
    { event_index: 2, junme: 1, is_key: true, actual: 'E' },
    { event_index: 10, junme: 2, is_key: false, actual: 'S' },
  ],
}

const step = (index: number) => ({ snapshot: { kyoku: 1 }, mahgen: { players: [] }, analysis: index === 2 ? {} : null })

describe('tutorialStore replay + review flow', () => {
  beforeEach(() => {
    calls.length = 0
    responses.clear()
    responses.set('tutorial_replay_outline', outline)
    responses.set('tutorial_get_review', null)
    responses.set('tutorial_get_bot_reference', null)
    responses.set('tutorial_get_live_bot', null)
    responses.set('tutorial_replay_step', step(2))
    const backing = new Map<string, string>()
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => backing.get(k) ?? null,
      setItem: (k: string, v: string) => void backing.set(k, String(v)),
      removeItem: (k: string) => void backing.delete(k),
      clear: () => backing.clear(),
    })
  })

  it('openGame loads outline + cached doc and jumps to the first decision', async () => {
    const { useTutorialStore } = await freshStore()
    await useTutorialStore.getState().openGame('01JGAME')
    const s = useTutorialStore.getState()
    expect(s.outline).toEqual(outline)
    expect(s.stepIndex).toBe(2)
    expect(calls.map((c) => c.cmd)).toContain('tutorial_replay_outline')
    expect(calls).toContainEqual({
      cmd: 'tutorial_replay_step',
      args: { gameId: '01JGAME', eventIndex: 2 },
    })
  })

  it('steps clamp to the stream bounds and fetch each position', async () => {
    const { useTutorialStore } = await freshStore()
    await useTutorialStore.getState().openGame('01JGAME')
    calls.length = 0
    await useTutorialStore.getState().setStepIndex(999)
    expect(useTutorialStore.getState().stepIndex).toBe(19)
    await useTutorialStore.getState().stepBy(-5)
    expect(useTutorialStore.getState().stepIndex).toBe(14)
    expect(calls).toContainEqual({
      cmd: 'tutorial_replay_step',
      args: { gameId: '01JGAME', eventIndex: 14 },
    })
  })

  it('generate deletes the cache first when regenerating, then calls with coverage', async () => {
    responses.set('tutorial_get_review', { game_id: '01JGAME', turns: [] })
    responses.set('tutorial_delete_review', true)
    responses.set('tutorial_generate_review', { game_id: '01JGAME', turns: [], overview: 'ok' })
    const { useTutorialStore } = await freshStore()
    await useTutorialStore.getState().openGame('01JGAME')
    calls.length = 0
    await useTutorialStore.getState().generate()
    const s = useTutorialStore.getState()
    expect(s.generating).toBe(false)
    expect(s.doc).toEqual({ game_id: '01JGAME', turns: [], overview: 'ok' })
    expect(calls.map((c) => c.cmd)).toEqual([
      'tutorial_delete_review',
      'tutorial_generate_review',
    ])
    expect(calls[1].args).toEqual({ gameId: '01JGAME', coverage: 'key' })
  })

  it('generate surfaces backend errors as generateError', async () => {
    responses.delete('tutorial_get_review')
    responses.set('tutorial_generate_review', null) // fall through → throw below
    const { invoke } = await import('@/lib/tauri')
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      calls.push({ cmd, args: {} })
      if (cmd === 'tutorial_generate_review') throw new Error('outbound not allowed')
      if (responses.has(cmd)) return responses.get(cmd)
      throw new Error(`unexpected command: ${cmd}`)
    })
    const { useTutorialStore } = await freshStore()
    await useTutorialStore.getState().openGame('01JGAME')
    await useTutorialStore.getState().generate()
    const s = useTutorialStore.getState()
    expect(s.generating).toBe(false)
    expect(s.generateError).toContain('outbound not allowed')
  })

  it('persists the coverage preference', async () => {
    const { useTutorialStore } = await freshStore()
    expect(useTutorialStore.getState().coverage).toBe('key')
    useTutorialStore.getState().setCoverage('all')
    expect(localStorage.getItem('akagi.tutorial.coverage')).toBe('all')
    const reloaded = await freshStore()
    expect(reloaded.useTutorialStore.getState().coverage).toBe('all')
  })
})

describe('tutorialStore coach auto state (D-0014)', () => {
  beforeEach(() => {
    calls.length = 0
    responses.clear()
  })

  it('auto turn lifecycle: pending on draw, doc on event, reset on new game', async () => {
    const { useTutorialStore } = await freshStore()
    const st = () => useTutorialStore.getState().coach

    // New draw (bot-response heard by the bridge):
    useTutorialStore.getState().coachAutoTurnStart()
    expect(st().autoPending).toBe(true)
    expect(st().autoDoc).toBeNull()

    // tutorial-live-turn arrives:
    useTutorialStore.getState().setAutoDoc({
      junme: 4,
      recommended: 'dahai:S',
      numbers: { shanten: 2, win_rate: null, deal_in_rate: null, avg_score: null },
      explanation_zh: '切孤张字牌。',
      headline: '防守',
    })
    expect(st().autoPending).toBe(false)
    expect(st().autoDoc?.explanation_zh).toBe('切孤张字牌。')

    // Next draw retires the old doc; start_game clears everything.
    useTutorialStore.getState().coachAutoTurnStart()
    expect(st().autoDoc).toBeNull()
    useTutorialStore.getState().setAutoDoc({
      junme: 5,
      recommended: 'dahai:P',
      numbers: { shanten: 1, win_rate: null, deal_in_rate: null, avg_score: null },
      explanation_zh: '第二巡。',
      headline: null,
    })
    useTutorialStore.getState().coachReset()
    expect(st().autoDoc).toBeNull()
    expect(st().autoPending).toBe(false)
    expect(st().muted).toBe(false)
  })
})

describe('llm log', () => {
  it('appends with increasing ids and caps the history at 50', async () => {
    const { useTutorialStore } = await freshStore()
    const st = () => useTutorialStore.getState()
    for (let i = 0; i < 60; i++) {
      st().appendLlmLog({
        ts: Date.now(),
        junme: i + 1,
        model: 'm',
        latencyMs: null,
        source: 'auto',
        ok: true,
      })
    }
    const log = st().llmLog
    expect(log).toHaveLength(50)
    // The cap keeps the newest 50 — junme 11..60 — and ids stay increasing.
    expect(log[0].junme).toBe(11)
    expect(log[49].junme).toBe(60)
    expect(log[49].id).toBeGreaterThan(log[0].id)
  })
})
