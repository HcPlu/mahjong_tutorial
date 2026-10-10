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

const config = (autoplayEnabled: boolean) => ({
  overlay: { enabled: true, top_n: 3, opacity: 0.95, always_on_top: true },
  autoplay: { enabled: autoplayEnabled },
})

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

  it('reverts the optimistic state when the command fails', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_config') return Promise.resolve(config(false))
      return Promise.reject(new Error('persist failed'))
    })
    render(<Overlay />)
    const btn = await waitFor(() =>
      screen.getByRole('button', { name: 'overlay.auto_on_title' }),
    )
    fireEvent.click(btn)
    await waitFor(() => expect(btn.getAttribute('aria-pressed')).toBe('false'))
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
