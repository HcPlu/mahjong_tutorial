import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'

import { Overlay } from './Overlay'

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
}))

// The overlay only needs `t` as an identity function — asserting on raw keys
// keeps the test independent of copy changes across the four locales.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock('@/lib/tauri', () => ({
  invoke: (...args: unknown[]) => invokeMock(...(args as never[])),
  listen: (...args: unknown[]) => listenMock(...(args as never[])),
}))

const config = (autoplayEnabled: boolean, tutorial?: unknown) => ({
  overlay: { enabled: true, top_n: 3, opacity: 0.95, always_on_top: true },
  autoplay: { enabled: autoplayEnabled },
  tutorial,
})

// Fully consented + configured endpoint with coach auto on — the state
// where the explain panel actually renders explanations.
const TUTORIAL_AUTO_ON = {
  enabled: true,
  upload_consent: true,
  llm: {
    base_url: 'https://example.com/v1',
    api_key: '',
    model: 'some-model',
    request_timeout_ms: 60000,
  },
  coach: { enabled: true, max_per_turn: 1, deep_dive_button: true, auto_policy: 'all' },
}

const TURN_DOC = {
  junme: 7,
  recommended: '5s',
  numbers: { shanten: 1, win_rate: 0.3, deal_in_rate: 0.05, avg_score: 1200 },
  explanation_zh: '切五索推进向听，保留安全余裕。',
  headline: '推进手牌',
}

function capturedListener(event: string): (payload: unknown) => void {
  const call = listenMock.mock.calls.find(([ev]) => ev === event)
  if (!call) throw new Error(`no listener captured for ${event}`)
  return call[1] as (payload: unknown) => void
}

describe('Overlay autoplay toggle', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    listenMock.mockReset()
    listenMock.mockImplementation(() => Promise.resolve(() => {}))
  })

  it('reflects the persisted autoplay state on mount', async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'get_config' ? Promise.resolve(config(true)) : Promise.resolve(),
    )
    render(<Overlay />)
    const btn = await waitFor(() =>
      screen.getByRole('button', { name: 'overlay.auto_off_title' }),
    )
    expect(btn.getAttribute('aria-pressed')).toBe('true')
  })

  it('applies the toggle through set_autoplay_enabled', async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'get_config' ? Promise.resolve(config(false)) : Promise.resolve(),
    )
    render(<Overlay />)
    const btn = await waitFor(() =>
      screen.getByRole('button', { name: 'overlay.auto_on_title' }),
    )
    fireEvent.click(btn)
    expect(invokeMock).toHaveBeenCalledWith('set_autoplay_enabled', { enabled: true })
    await waitFor(() => expect(btn.getAttribute('aria-pressed')).toBe('true'))
  })

  it('converges on the backend state when the command fails', async () => {
    // Toggle write fails while the backend still has autoplay enabled
    // (e.g. the persist raced another window) — the button must settle
    // on the truth, not blindly flip back to a lie.
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_config') return Promise.resolve(config(true))
      return Promise.reject(new Error('persist failed'))
    })
    render(<Overlay />)
    const btn = await waitFor(() =>
      screen.getByRole('button', { name: 'overlay.auto_off_title' }),
    )
    fireEvent.click(btn)
    expect(invokeMock).toHaveBeenCalledWith('set_autoplay_enabled', { enabled: false })
    await waitFor(() => expect(btn.getAttribute('aria-pressed')).toBe('true'))
  })

  it('re-syncs when a Settings save broadcasts overlay-config', async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'get_config' ? Promise.resolve(config(false)) : Promise.resolve(),
    )
    render(<Overlay />)
    await waitFor(() =>
      screen.getByRole('button', { name: 'overlay.auto_on_title' }),
    )

    // Settings saved with autoplay enabled → update_config broadcasts
    // overlay-config, and the overlay refetches the whole config.
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'get_config' ? Promise.resolve(config(true)) : Promise.resolve(),
    )
    capturedListener('overlay-config')(null)
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: 'overlay.auto_off_title' }).getAttribute('aria-pressed'),
      ).toBe('true'),
    )
  })
})

describe('Overlay explain panel', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    listenMock.mockReset()
    listenMock.mockImplementation(() => Promise.resolve(() => {}))
  })

  it('shows pending on the decision, then the explanation when it lands', async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'get_config' ? Promise.resolve(config(true, TUTORIAL_AUTO_ON)) : Promise.resolve(),
    )
    render(<Overlay />)
    await waitFor(() =>
      expect(
        screen
          .getByRole('button', { name: 'overlay.explain_off_title' })
          .getAttribute('aria-pressed'),
      ).toBe('true'),
    )

    // A bot reply marks a fresh decision → generation in flight.
    capturedListener('bot-response')({ meta: {} })
    await waitFor(() =>
      expect(screen.getByTestId('overlay-explain-pending')).toBeTruthy(),
    )

    // The auto deep-dive arrives → its text replaces the pending state.
    capturedListener('tutorial-live-turn')({
      session_key: 's',
      junme: 7,
      model: 'some-model',
      doc: TURN_DOC,
    })
    await waitFor(() => expect(screen.getByTestId('overlay-explain-doc')).toBeTruthy())
    expect(screen.getByText('推进手牌')).toBeTruthy()
    expect(screen.getByText('切五索推进向听，保留安全余裕。')).toBeTruthy()
    expect(screen.queryByTestId('overlay-explain-pending')).toBeNull()

    // The next decision retires the shown doc back to the pending state —
    // a stale explanation must not mask the "generating…" indicator.
    capturedListener('bot-response')({ meta: {} })
    await waitFor(() => expect(screen.queryByTestId('overlay-explain-doc')).toBeNull())
    expect(screen.getByTestId('overlay-explain-pending')).toBeTruthy()
  })

  it('applies the explain toggle through set_coach_auto', async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'get_config' ? Promise.resolve(config(false)) : Promise.resolve(),
    )
    render(<Overlay />)
    const btn = await waitFor(() =>
      screen.getByRole('button', { name: 'overlay.explain_on_title' }),
    )
    expect(btn.getAttribute('aria-pressed')).toBe('false')
    fireEvent.click(btn)
    expect(invokeMock).toHaveBeenCalledWith('set_coach_auto', { enabled: true })
    await waitFor(() => expect(btn.getAttribute('aria-pressed')).toBe('true'))
  })

  it('shows the unconfigured hint instead of a spinner when outbound is off', async () => {
    // Auto on but no consent / endpoint: the backend silently skips every
    // turn — a pending spinner here would misread as "generation is slow".
    const unconfigured = {
      ...TUTORIAL_AUTO_ON,
      upload_consent: false,
    }
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'get_config' ? Promise.resolve(config(true, unconfigured)) : Promise.resolve(),
    )
    render(<Overlay />)
    await waitFor(() => expect(screen.getByTestId('overlay-explain-panel')).toBeTruthy())
    capturedListener('bot-response')({ meta: {} })
    await waitFor(() =>
      expect(screen.getByText('overlay.explain_unconfigured')).toBeTruthy(),
    )
    expect(screen.queryByTestId('overlay-explain-pending')).toBeNull()
  })
})
