import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'

import { GenerationParams } from './GenerationParams'
import { useConfigStore } from '@/stores/configStore'
import type { AppConfig } from '@/types'

vi.mock('@/lib/tauri', () => ({
  invoke: vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args })
    if (cmd === 'get_config') return structuredClone(baseConfig)
    return null
  }),
}))
const calls: { cmd: string; args?: Record<string, unknown> }[] = []

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}))

const baseConfig: AppConfig = {
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
      model: 'glm-5.3-flashx',
      request_timeout_ms: 300000,
      reasoning_effort: 'low',
    },
    coach: { enabled: false, max_per_turn: 1, deep_dive_button: true },
  },
}

describe('GenerationParams', () => {
  beforeEach(() => {
    calls.length = 0
    useConfigStore.getState().setConfig(structuredClone(baseConfig))
  })

  it('shows the current model, thinking and endpoint', () => {
    render(<GenerationParams />)
    expect(screen.getByText('glm-5.3-flashx')).toBeTruthy()
    expect(screen.getByText('low')).toBeTruthy()
    expect(
      screen.getByText(/open\.bigmodel\.cn.*300s/),
    ).toBeTruthy()
  })

  it('typing a custom model write-throughs update_config and the store', async () => {
    useConfigStore
      .getState()
      .setConfig(structuredClone({ ...baseConfig, tutorial: { ...baseConfig.tutorial!, llm: { ...baseConfig.tutorial!.llm, model: 'my-custom-model' } } }))
    render(<GenerationParams />)
    const input = screen.getByPlaceholderText('model id')
    expect((input as HTMLInputElement).value).toBe('my-custom-model')
    fireEvent.change(input, { target: { value: 'glm-4.7-flash' } })
    fireEvent.blur(input)

    await waitFor(() => {
      const upd = calls.find((c) => c.cmd === 'update_config')
      expect(upd).toBeTruthy()
      const next = upd!.args!.newConfig as AppConfig
      expect(next.tutorial?.llm.model).toBe('glm-4.7-flash')
      // untouched fields survive the read-modify-write
      expect(next.tutorial?.llm.reasoning_effort).toBe('low')
      expect(next.tutorial?.upload_consent).toBe(true)
    })
    expect(
      useConfigStore.getState().config?.tutorial?.llm.model,
    ).toBe('glm-4.7-flash')
  })
})
