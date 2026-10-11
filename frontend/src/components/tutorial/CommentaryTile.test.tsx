// CommentaryTile auto-mode states (D-0014): pending spinner while the
// background generation races the discard window, the doc box the moment
// `tutorial-live-turn` lands, suppression when muted or auto is off, and
// the auto doc winning over a stale manual deepDoc.

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'

import { CommentaryTile } from './CommentaryTile'
import type { AppConfig } from '@/types'
import type { TurnDoc } from '@/components/tutorial/types'
import { useConfigStore } from '@/stores/configStore'
import { useTutorialStore } from '@/stores/tutorialStore'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}))

const baseConfig = (autoPolicy: 'off' | 'all', coachEnabled = true): AppConfig =>
  ({
    tutorial: {
      enabled: true,
      upload_consent: true,
      llm: {
        base_url: 'https://example.com/v1',
        api_key: 'k',
        model: 'm',
        request_timeout_ms: 60_000,
      },
      coach: {
        enabled: coachEnabled,
        max_per_turn: 1,
        deep_dive_button: true,
        auto_policy: autoPolicy,
      },
    },
  }) as unknown as AppConfig

const doc = (text: string): TurnDoc => ({
  junme: 3,
  recommended: 'dahai:E',
  numbers: { shanten: 1, win_rate: 0.2, deal_in_rate: 0.01, avg_score: null },
  explanation_zh: text,
  headline: '要点',
})

type CoachState = ReturnType<typeof useTutorialStore.getState>['coach']

function setCoach(partial: Partial<CoachState>) {
  useTutorialStore.setState({ coach: { ...useTutorialStore.getState().coach, ...partial } })
}

describe('CommentaryTile auto mode', () => {
  beforeEach(() => {
    useTutorialStore.getState().coachReset()
    useConfigStore.setState({ config: null })
  })

  it('shows the pending spinner while auto generation is in flight', () => {
    useConfigStore.setState({ config: baseConfig('all') })
    setCoach({ autoPending: true, autoDoc: null })
    render(<CommentaryTile bp="md" />)
    expect(screen.getByTestId('coach-auto-pending')).toBeTruthy()
    expect(screen.queryByTestId('coach-auto-doc')).toBeNull()
  })

  it('renders the auto doc once it arrives (tile names translated)', () => {
    useConfigStore.setState({ config: baseConfig('all') })
    setCoach({ autoPending: false, autoDoc: doc('切五索保留搭子') })
    render(<CommentaryTile bp="md" />)
    expect(screen.getByTestId('coach-auto-doc')).toBeTruthy()
    expect(screen.getByText('切五索保留搭子')).toBeTruthy()
    expect(screen.queryByTestId('coach-auto-pending')).toBeNull()
  })

  it('stays silent while muted even with a fresh auto doc', () => {
    useConfigStore.setState({ config: baseConfig('all') })
    setCoach({ muted: true, autoDoc: doc('不该出现') })
    render(<CommentaryTile bp="md" />)
    expect(screen.queryByTestId('coach-auto-doc')).toBeNull()
    expect(screen.queryByTestId('coach-auto-pending')).toBeNull()
  })

  it('auto off: no spinner/doc even with stale pending state, deepDoc still shows', () => {
    useConfigStore.setState({ config: baseConfig('off') })
    setCoach({ autoPending: true, autoDoc: doc('不该出现'), deepDoc: doc('手动讲解') })
    render(<CommentaryTile bp="md" />)
    expect(screen.queryByTestId('coach-auto-pending')).toBeNull()
    expect(screen.queryByTestId('coach-auto-doc')).toBeNull()
    expect(screen.getByText('手动讲解')).toBeTruthy()
  })

  it('a fresh auto doc replaces a stale manual deepDoc', () => {
    useConfigStore.setState({ config: baseConfig('all') })
    setCoach({ autoDoc: doc('自动讲解'), deepDoc: doc('旧的手动讲解') })
    render(<CommentaryTile bp="md" />)
    expect(screen.getByText('自动讲解')).toBeTruthy()
    expect(screen.queryByText('旧的手动讲解')).toBeNull()
  })
})
