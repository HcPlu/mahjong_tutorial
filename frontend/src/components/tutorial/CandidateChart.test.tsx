import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'

import { CandidateChart } from './CandidateChart'
import type { AnalysisResult, DiscardCandidate } from '@/types'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}))

// Minimal Hand13Result carrying only what the chart reads.
function cand(
  discard: string,
  over: Partial<DiscardCandidate['result']>,
): DiscardCandidate {
  return {
    discard,
    result: {
      shanten: 3,
      waits_total: 0,
      mixed_waits_score: 0,
      avg_agari_rate: 0,
      ...over,
    } as DiscardCandidate['result'],
  }
}

function analysis(maintain: DiscardCandidate[], risk: number[] = []) {
  return {
    state: 'discard14',
    shanten: 3,
    hand13: null,
    hand14: { shanten: 3, maintain, backwards: [] },
    opponents: [],
    mixed_risk: risk,
    best_attack_discard: maintain[0]?.discard ?? null,
    best_defence_discard: null,
  } as unknown as AnalysisResult
}

describe('CandidateChart', () => {
  it('separates ukeire ties with the engine score and collapses dead metrics to —', () => {
    // Real shape from game 01M4KAYHA junme 3: P/C tie at 53 ukeire, far
    // hand so win rate has no signal, no tenpai threat so risk is flat 0.
    const a = analysis([
      cand('P', { waits_total: 53, mixed_waits_score: 88.861 }),
      cand('C', { waits_total: 53, mixed_waits_score: 88.861 }),
      cand('8s', { waits_total: 42, mixed_waits_score: 83.174 }),
    ])
    const { container } = render(<CandidateChart analysis={a} actual="8s" />)
    const rows = container.querySelectorAll('.flex.items-center.gap-3')
    expect(rows.length).toBe(3)
    // Both metrics are dead (all zeros) → em dashes, no % noise.
    const dashes = screen.getAllByText('—')
    expect(dashes.length).toBeGreaterThanOrEqual(6) // 2 per row
    expect(screen.queryByText('0%')).toBeNull()
    expect(screen.queryByText('0.0%')).toBeNull()
    // Score column differentiates the tied pair (two rows share 88.9).
    expect(screen.getAllByText('88.9').length).toBe(2)
    expect(screen.getByText('83.2')).toBeTruthy()
    // Badges.
    expect(screen.getByText('tutorial.actual_pick')).toBeTruthy()
    expect(screen.getByText('tutorial.engine_best')).toBeTruthy()
  })

  it('shows bot probabilities and the chosen badge when the model picked a row', () => {
    const a = analysis([
      cand('P', { waits_total: 53, mixed_waits_score: 88.8 }),
      cand('C', { waits_total: 53, mixed_waits_score: 88.8 }),
    ])
    render(
      <CandidateChart
        analysis={a}
        actual={null}
        botTurn={{
          junme: 1,
          action: 'dahai:C',
          source: 'model',
          picks: [
            { tile: 'C', label: 'Discard', prob: 0.55, chosen: true },
            { tile: 'P', label: 'Discard', prob: 0.3 },
          ],
        }}
      />,
    )
    expect(screen.getAllByText('tutorial.bot_prob').length).toBe(2) // both rows carry a bot prob
    // Chosen == top here, so only the chosen badge renders (not both).
    expect(screen.getAllByText('tutorial.bot_chosen').length).toBe(1)
    expect(screen.queryByText('tutorial.bot_pick')).toBeNull()
    // Turn-level provenance badge.
    expect(screen.getByText('tutorial.bot_source_model')).toBeTruthy()
  })

  it('shows top and chosen badges separately when a rule overrode the model', () => {
    const a = analysis([
      cand('P', { waits_total: 53, mixed_waits_score: 88.8 }),
      cand('C', { waits_total: 53, mixed_waits_score: 88.8 }),
    ])
    render(
      <CandidateChart
        analysis={a}
        actual={null}
        botTurn={{
          junme: 1,
          action: 'dahai:C',
          source: 'rule_overridden',
          picks: [
            { tile: 'P', label: 'Discard', prob: 0.6 },
            { tile: 'C', label: 'Discard', prob: 0.3, chosen: true },
          ],
        }}
      />,
    )
    expect(screen.getAllByText('tutorial.bot_pick').length).toBe(1) // P: the net's rank-1
    expect(screen.getAllByText('tutorial.bot_chosen').length).toBe(1) // C: what was played
    expect(screen.getByText('tutorial.bot_source_rule_overridden')).toBeTruthy()
  })

  it('marks the forced tile without inventing a probability on rule-forced turns', () => {
    const a = analysis([
      cand('P', { waits_total: 53, mixed_waits_score: 88.8 }),
      cand('C', { waits_total: 53, mixed_waits_score: 88.8 }),
    ])
    render(
      <CandidateChart
        analysis={a}
        actual={null}
        botTurn={{ junme: 1, action: 'dahai:P', source: 'rule_forced', picks: [] }}
      />,
    )
    // No distribution exists → no bot prob on any row, but the forced
    // tile is still marked as what the bot played.
    expect(screen.queryByText('tutorial.bot_prob')).toBeNull()
    expect(screen.getAllByText('tutorial.bot_chosen').length).toBe(1)
    expect(screen.getByText('tutorial.bot_source_rule_forced')).toBeTruthy()
  })

  it('renders live metrics as percentages when the turn has signal', () => {
    const a = analysis(
      [
        cand('5s', { waits_total: 8, mixed_waits_score: 90, avg_agari_rate: 42 }),
        cand('E', { waits_total: 4, mixed_waits_score: 70, avg_agari_rate: 31 }),
      ],
      Array(34).fill(0).map((_, i) => (i === 22 ? 9.1 : 2)), // 5s risk 9.1%
    )
    render(<CandidateChart analysis={a} actual={null} />)
    expect(screen.getByText('42%')).toBeTruthy()
    expect(screen.getByText('9.1%')).toBeTruthy()
    // Live dimensions never render the dead dash.
    expect(screen.queryByText('—')).toBeNull()
  })
})
