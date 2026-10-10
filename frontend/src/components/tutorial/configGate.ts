// Consent/config gating helpers for the tutorial UI. Mirror the backend
// `TutorialConfig::outbound_allowed()` logic (D-0007) — display-only:
// the backend re-checks before any request leaves the machine.

import type { AppConfig, TutorialConfig } from '@/types'

export const DEFAULT_TUTORIAL_CONFIG: TutorialConfig = {
  enabled: true,
  upload_consent: false,
  llm: { base_url: '', api_key: '', model: '', request_timeout_ms: 60_000 },
  coach: { enabled: false, max_per_turn: 1, deep_dive_button: true },
}

/** The tutorial section of a config payload, with defaults for legacy
 *  payloads that predate the section. */
export function getTutorialConfig(cfg: AppConfig | null | undefined): TutorialConfig {
  return cfg?.tutorial ?? DEFAULT_TUTORIAL_CONFIG
}

/** True when the LLM endpoint looks configured (URL + model).
 *
 *  Deliberately does NOT require `api_key`: the backend's
 *  `effective_api_key()` lets `TUTORIAL_LLM_API_KEY` override the stored
 *  key, and the frontend cannot see env vars. An env-key setup must keep
 *  the buttons enabled — the backend stays authoritative and returns a
 *  clear error if no key exists at all. */
export function llmConfigured(cfg: TutorialConfig): boolean {
  return cfg.llm.base_url.trim() !== '' && cfg.llm.model.trim() !== ''
}

/** Mirror of `TutorialConfig::outbound_allowed()` — gates the generate /
 *  deep-dive buttons. */
export function outboundAllowed(cfg: TutorialConfig): boolean {
  return cfg.enabled && cfg.upload_consent && llmConfigured(cfg)
}
