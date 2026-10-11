// Generation parameters shown inline on the tutorial replay page: the
// model and reasoning effort the next prompt upload will use, editable
// right where generation happens. Changes write straight through to the
// shared AppConfig (update_config → toml_edit persist) — the backend
// reads the config at request time, so a new model takes effect on the
// very next click, no restart needed.

import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { LLM_PRESETS } from '@/components/tutorial/llmPresets'
import { getTutorialConfig } from '@/components/tutorial/configGate'
import { invoke } from '@/lib/tauri'
import { useConfigStore } from '@/stores/configStore'
import type { AppConfig } from '@/types'

/** Fixed model menu: the GLM flash lineup this fork tunes against plus
 *  the preset models. Anything else configured counts as "custom" and
 *  switches the control to a free-text input (prefilled). */
const FIXED_MODELS: string[] = [
  'glm-5.3-flashx',
  'glm-5.3-flash',
  'glm-4.7-flash',
  'glm-4.5-flash',
  ...LLM_PRESETS.map((p) => p.model),
]

const CUSTOM = '__custom__'
const THINKING_DEFAULT = '__default__'

export function GenerationParams() {
  const { t } = useTranslation()
  const config = useConfigStore((s) => s.config)
  const setConfig = useConfigStore((s) => s.setConfig)
  const cfg = getTutorialConfig(config)
  const [busy, setBusy] = useState(false)
  const model = cfg.llm.model

  const writeThrough = async (patch: {
    model?: string
    reasoning_effort?: string | null
  }) => {
    if (busy) return
    setBusy(true)
    try {
      // Read-modify-write the live config so we never clobber other
      // sections with a stale copy.
      const live = await invoke<AppConfig>('get_config')
      const next: AppConfig = {
        ...live,
        tutorial: {
          ...getTutorialConfig(live),
          llm: {
            ...getTutorialConfig(live).llm,
            ...(patch.model !== undefined ? { model: patch.model } : {}),
            ...(patch.reasoning_effort !== undefined
              ? { reasoning_effort: patch.reasoning_effort }
              : {}),
          },
        },
      }
      await invoke('update_config', { newConfig: next })
      setConfig(next)
    } catch (e) {
      toast.error(String(e))
    } finally {
      setBusy(false)
    }
  }

  const isCustom = !!model && !FIXED_MODELS.includes(model)
  const value = isCustom || !model ? CUSTOM : model

  return (
    <div className="flex flex-wrap items-center gap-2 text-xs">
      <span className="text-muted-foreground">{t('tutorial.param_model')}</span>
      <Select
        value={value}
        onValueChange={(v) => {
          if (v !== CUSTOM) void writeThrough({ model: v })
        }}
      >
        <SelectTrigger className="h-8 w-44 text-xs" disabled={busy}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {FIXED_MODELS.map((m) => (
            <SelectItem key={m} value={m} className="font-mono text-xs">
              {m}
            </SelectItem>
          ))}
          <SelectItem value={CUSTOM}>{t('tutorial.param_custom')}</SelectItem>
        </SelectContent>
      </Select>
      {value === CUSTOM && (
        // Uncontrolled + key: out-of-band config changes (Settings save)
        // remount the input with the new value — no state syncing.
        <Input
          key={model}
          className="h-8 w-44 font-mono text-xs"
          placeholder="model id"
          defaultValue={model}
          disabled={busy}
          onBlur={(e) => {
            const v = e.target.value.trim()
            if (v && v !== model) void writeThrough({ model: v })
          }}
        />
      )}
      <span className="ml-2 text-muted-foreground">{t('tutorial.param_thinking')}</span>
      <Select
        value={cfg.llm.reasoning_effort ?? THINKING_DEFAULT}
        onValueChange={(v) =>
          void writeThrough({
            reasoning_effort: v === THINKING_DEFAULT ? null : v,
          })
        }
      >
        <SelectTrigger className="h-8 w-28 text-xs" disabled={busy}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={THINKING_DEFAULT}>{t('tutorial.thinking_default')}</SelectItem>
          <SelectItem value="low">low</SelectItem>
          <SelectItem value="medium">medium</SelectItem>
          <SelectItem value="high">high</SelectItem>
        </SelectContent>
      </Select>
      <span className="ml-auto font-mono text-[11px] text-muted-foreground/70">
        {cfg.llm.base_url || '—'} · {Math.round(cfg.llm.request_timeout_ms / 1000)}s
      </span>
    </div>
  )
}
