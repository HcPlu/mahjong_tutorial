// Provider presets for the tutorial LLM card (Settings). Each fills
// `base_url` + `model`; the API key is always entered by the user and
// never shipped. Prices surveyed 2026-10 (docs/DECISIONS.md D-0012);
// ids are editable afterwards — presets are just typing shortcuts.

export type LlmPreset = {
  id: string
  /** i18n key suffix under settings.tutorial.preset_* */
  labelKey: string
  baseUrl: string
  model: string
}

export const LLM_PRESETS: LlmPreset[] = [
  {
    id: 'glm-flash',
    labelKey: 'settings.tutorial.preset_glm',
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    model: 'glm-4.7-flash',
  },
  {
    id: 'qwen-flash',
    labelKey: 'settings.tutorial.preset_qwen',
    baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    model: 'qwen-flash',
  },
  {
    id: 'deepseek',
    labelKey: 'settings.tutorial.preset_deepseek',
    baseUrl: 'https://api.deepseek.com/v1',
    model: 'deepseek-flash',
  },
  {
    id: 'kimi',
    labelKey: 'settings.tutorial.preset_kimi',
    baseUrl: 'https://api.moonshot.cn/v1',
    model: 'kimi-k2.6',
  },
]
