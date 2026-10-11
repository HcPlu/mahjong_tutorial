import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'

import { TutorialLlmCard } from './TutorialLlmCard'
import type { AppConfig } from '@/types'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}))

const baseDraft: AppConfig = {
  general: { first_run_completed: true, developer_mode: false },
  logging: { dir: './logs', level: 'info', all_level: 'info' },
  platform: { kind: 'Majsoul' },
  proxy: { enabled: false, addr: '', ca_dir: '', block_telemetry: false },
  bot: {
    enabled: false,
    active_4p: '',
    active_3p: '',
    auto_sync: false,
    dir: 'mjai_bot',
    api: {
      enabled: false,
      base_url: '',
      key: '',
      model_4p: '',
      model_3p: '',
      proxy_enabled: false,
      proxy: '',
      react_timeout_ms: 3000,
    },
  },
  capture: {
    mode: 'chromium',
    chromium: {
      executable: '',
      user_data_dir: '',
      start_url: '',
      cft_channel: 'stable',
      force_cft: false,
      extra_args: [],
    },
  },
  autoplay: {
    enabled: false,
    majsoul: {
      pre_click_delay_min_ms: 1,
      pre_click_delay_max_ms: 1,
      inter_click_delay_ms: 1,
      hover_delay_ms: 1,
      click_hold_ms: 1,
      verify_input_ms: 1,
      click_retries: 1,
      reload_after_failures: 1,
      dealer_first_discard_extra_delay_ms: 1,
    },
    delay: {
      mode: 'lua',
      min_delay_ms: 1,
      min_button_delay_ms: 1,
      distribution: 'log_normal',
      lognormal: {},
      bank_on_long_thought: false,
      riichi_extra_ms: 0,
      kan_extra_ms: 0,
      safety_margin_ms: 0,
      bank_use_fraction: 0,
      bank_max_single_ms: 0,
      no_budget_cap_ms: 0,
    },
  },
  overlay: {
    enabled: false,
    top_n: 3,
    opacity: 0.95,
    always_on_top: true,
  },
  network: { github_mirror_mode: 'auto', github_custom_mirror: '' },
  tutorial: {
    enabled: true,
    upload_consent: true,
    llm: {
      base_url: 'https://open.bigmodel.cn/api/paas/v4',
      api_key: '',
      model: 'glm-4.7-flash',
      request_timeout_ms: 60000,
    },
    coach: { enabled: false, max_per_turn: 1, deep_dive_button: true, auto_policy: 'off' },
  },
}

describe('TutorialLlmCard', () => {
  it('renders the coach toggle reflecting the draft', () => {
    render(<TutorialLlmCard draft={baseDraft} setDraft={() => {}} />)
    const box = screen.getByRole('checkbox', { name: 'settings.tutorial.coach' })
    expect((box as HTMLInputElement).checked).toBe(false)
  })

  it('toggling the coach switch patches only [tutorial.coach] enabled', () => {
    const setDraft = vi.fn()
    render(<TutorialLlmCard draft={baseDraft} setDraft={setDraft} />)
    fireEvent.click(screen.getByRole('checkbox', { name: 'settings.tutorial.coach' }))

    expect(setDraft).toHaveBeenCalledTimes(1)
    const next = setDraft.mock.calls[0][0] as AppConfig
    expect(next.tutorial?.coach.enabled).toBe(true)
    // read-modify-write: every other tutorial field survives
    expect(next.tutorial?.coach.auto_policy).toBe('off')
    expect(next.tutorial?.coach.deep_dive_button).toBe(true)
    expect(next.tutorial?.upload_consent).toBe(true)
    expect(next.tutorial?.llm.model).toBe('glm-4.7-flash')
    // and the rest of the config passes through untouched
    expect(next.bot).toEqual(baseDraft.bot)
  })
})
