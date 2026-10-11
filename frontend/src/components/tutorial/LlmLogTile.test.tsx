import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'

import { LlmLogTile } from './LlmLogTile'
import { useTutorialStore, type LlmLogEntry } from '@/stores/tutorialStore'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}))

const entries = (...e: Array<Partial<LlmLogEntry>>): LlmLogEntry[] =>
  e.map((x, i) => ({
    id: i + 1,
    ts: 1791720000000 + i * 1000,
    junme: null,
    model: null,
    latencyMs: null,
    source: 'auto',
    ok: true,
    ...x,
  }))

describe('LlmLogTile', () => {
  beforeEach(() => {
    useTutorialStore.setState({ llmLog: [] })
  })

  it('shows the empty state before any explanation lands', () => {
    render(<LlmLogTile bp="lg" />)
    expect(screen.getByText('tutorial.llm_log_empty')).toBeTruthy()
  })

  it('renders entries newest-first with latency, model and headline', () => {
    useTutorialStore.setState({
      llmLog: entries(
        { junme: 7, model: 'glm-5.3-flashx', latencyMs: 3400, headline: '推进向听' },
        { junme: 8, latencyMs: 40, headline: '缓存命中例' },
      ),
    })
    render(<LlmLogTile bp="lg" />)
    expect(screen.getByText('3.4s')).toBeTruthy()
    expect(screen.getByText('glm-5.3-flashx')).toBeTruthy()
    // Newest (junme 8) renders above the older entry.
    expect(
      (screen.getByText('缓存命中例') as HTMLElement).compareDocumentPosition(
        screen.getByText('推进向听') as HTMLElement,
      ) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()
  })

  it('renders failures with their error', () => {
    useTutorialStore.setState({
      llmLog: entries({ source: 'manual', ok: false, error: 'LLM request failed' }),
    })
    render(<LlmLogTile bp="lg" />)
    expect(screen.getByText('tutorial.llm_log_manual')).toBeTruthy()
    expect(screen.getByText('LLM request failed')).toBeTruthy()
  })
})
