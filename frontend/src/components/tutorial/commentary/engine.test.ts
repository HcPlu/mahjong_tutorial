import { describe, expect, it } from 'vitest'

import { evaluate } from './engine'
import type { AnalysisResult, GameStateSnapshot, ShowMeta } from '@/types'

// Fixtures carry only the fields the rules read — everything else is
// irrelevant to the engine and omitted via casts.
const snap = (over: Partial<GameStateSnapshot>) =>
  ({ our_seat: 0, players: [], ...over }) as GameStateSnapshot

const analysis = (over: Partial<AnalysisResult>) =>
  ({
    state: 'wait13',
    shanten: 1,
    hand13: null,
    hand14: null,
    opponents: [],
    mixed_risk: [],
    best_attack_discard: null,
    best_defence_discard: null,
    ...over,
  }) as AnalysisResult

const show = (items: ShowMeta['items']) => ({ items }) as ShowMeta

describe('commentary engine rules', () => {
  it('fires the tenpai rule on a 13-tile tenpai with waits', () => {
    const line = evaluate(
      analysis({
        shanten: 0,
        hand13: {
          waits: [
            { tile: '3s', left: 3, agari_rate: 40 },
            { tile: '6s', left: 4, agari_rate: 35 },
          ],
          waits_total: 7,
        } as AnalysisResult['hand13'],
      }),
      null,
      null,
    )
    expect(line?.ruleId).toBe('tenpai')
    expect(line?.params.n).toBe(7)
    expect(line?.tiles).toEqual(['3s', '6s'])
  })

  it('does not fire tenpai in the 14-tile discard state', () => {
    const line = evaluate(analysis({ state: 'discard14', shanten: 0 }), null, null)
    expect(line?.ruleId).not.toBe('tenpai')
  })

  it('fires defence when an opponent is riichi and names the safest tile', () => {
    const line = evaluate(
      analysis({ best_defence_discard: 'E', mixed_risk: Array(34).fill(0).map((_, i) => (i === 27 ? 1.8 : 9)) }),
      null,
      snap({
        players: [
          { seat: 0, riichi_declared: false },
          { seat: 2, riichi_declared: true },
        ] as GameStateSnapshot['players'],
      }),
    )
    expect(line?.ruleId).toBe('defence')
    expect(line?.params.tile).toBe('E')
    expect(line?.params.p).toBe('1.8')
  })

  it('ignores our own riichi for the defence rule', () => {
    const line = evaluate(
      analysis({ best_defence_discard: 'E' }),
      null,
      snap({
        our_seat: 0,
        players: [{ seat: 0, riichi_declared: true }] as GameStateSnapshot['players'],
      }),
    )
    expect(line).toBeNull()
  })

  it('fires bot-confidence when the top row dominates', () => {
    const line = evaluate(
      null,
      show([
        { label: 'Discard', pais: ['5s'], prob: 0.8 },
        { label: 'Discard', pais: ['1z'], prob: 0.1 },
      ]),
      null,
    )
    expect(line?.ruleId).toBe('bot-confident')
    expect(line?.params.action).toBe('Discard')
    expect(line?.params.prob).toBe(80)
  })

  it('skips bot-confidence when the top two are close', () => {
    const line = evaluate(
      null,
      show([
        { label: 'Discard', pais: ['5s'], prob: 0.5 },
        { label: 'Discard', pais: ['1z'], prob: 0.45 },
      ]),
      null,
    )
    expect(line).toBeNull()
  })

  it('defence outranks tenpai and bot confidence', () => {
    const line = evaluate(
      analysis({
        shanten: 0,
        best_defence_discard: 'E',
        hand13: { waits: [{ tile: '3s', left: 2, agari_rate: 1 }], waits_total: 2 } as AnalysisResult['hand13'],
      }),
      show([{ label: 'Discard', pais: ['5s'], prob: 0.9 }]),
      snap({
        players: [{ seat: 1, riichi_declared: true }] as GameStateSnapshot['players'],
      }),
    )
    expect(line?.ruleId).toBe('defence')
  })
})
