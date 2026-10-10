import { describe, expect, it } from 'vitest'

import {
  DEFAULT_TUTORIAL_CONFIG,
  getTutorialConfig,
  llmConfigured,
  outboundAllowed,
} from './configGate'
import type { AppConfig } from '@/types'

describe('configGate', () => {
  it('falls back to defaults for legacy payloads without a tutorial section', () => {
    const cfg = {} as AppConfig
    expect(getTutorialConfig(cfg)).toEqual(DEFAULT_TUTORIAL_CONFIG)
    expect(getTutorialConfig(null)).toEqual(DEFAULT_TUTORIAL_CONFIG)
  })

  it('outboundAllowed mirrors the backend gate: enabled AND consent AND configured', () => {
    const base = {
      ...DEFAULT_TUTORIAL_CONFIG,
      enabled: true,
      upload_consent: true,
      llm: {
        base_url: 'https://open.bigmodel.cn/api/paas/v4',
        api_key: 'k',
        model: 'glm-4.7-flash',
        request_timeout_ms: 60_000,
      },
    }
    expect(outboundAllowed(base)).toBe(true)
    expect(outboundAllowed({ ...base, upload_consent: false })).toBe(false)
    expect(outboundAllowed({ ...base, enabled: false })).toBe(false)
    expect(outboundAllowed({ ...base, llm: { ...base.llm, api_key: '' } })).toBe(false)
    expect(outboundAllowed({ ...base, llm: { ...base.llm, model: '  ' } })).toBe(false)
  })

  it('llmConfigured treats whitespace-only fields as empty', () => {
    expect(
      llmConfigured({
        ...DEFAULT_TUTORIAL_CONFIG,
        llm: { base_url: ' ', api_key: 'k', model: 'm', request_timeout_ms: 1 },
      }),
    ).toBe(false)
  })
})
