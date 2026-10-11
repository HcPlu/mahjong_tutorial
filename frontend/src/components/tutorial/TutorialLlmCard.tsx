// Settings card for the tutorial LLM (fork-only, D-0007): master
// switch, upload consent, provider presets, endpoint fields. Edits the
// shared draft — persistence goes through the normal Save flow
// (`update_config` → toml_edit merge on the backend). The API key is a
// password input and never leaves the config except to the configured
// endpoint.

import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Eye, EyeOff } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { LLM_PRESETS } from '@/components/tutorial/llmPresets'
import { DEFAULT_TUTORIAL_CONFIG } from '@/components/tutorial/configGate'
import type { AppConfig, TutorialConfig } from '@/types'

type Props = {
  draft: AppConfig
  setDraft: (next: AppConfig) => void
}

export function TutorialLlmCard({ draft, setDraft }: Props) {
  const { t } = useTranslation()
  const [showKey, setShowKey] = useState(false)

  const cfg: TutorialConfig = draft.tutorial ?? DEFAULT_TUTORIAL_CONFIG
  const patch = (p: Partial<TutorialConfig>) =>
    setDraft({ ...draft, tutorial: { ...cfg, ...p } })
  const patchLlm = (p: Partial<TutorialConfig['llm']>) =>
    patch({ llm: { ...cfg.llm, ...p } })
  const patchCoach = (p: Partial<TutorialConfig['coach']>) =>
    patch({ coach: { ...cfg.coach, ...p } })

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t('settings.tutorial.title')}</CardTitle>
      </CardHeader>
      <CardContent className="grid gap-4">
        <div className="flex items-center justify-between">
          <div>
            <span className="text-sm font-medium">
              {t('settings.tutorial.enabled')}
            </span>
            <p className="text-xs text-muted-foreground">
              {t('settings.tutorial.enabled_hint')}
            </p>
          </div>
          <input
            type="checkbox"
            className="h-4 w-4"
            checked={cfg.enabled}
            onChange={(e) => patch({ enabled: e.target.checked })}
            aria-label={t('settings.tutorial.enabled')}
          />
        </div>

        <div className="flex items-center justify-between">
          <div>
            <span className="text-sm font-medium">
              {t('settings.tutorial.consent')}
            </span>
            <p className="text-xs text-muted-foreground">
              {t('settings.tutorial.consent_hint')}
            </p>
          </div>
          <input
            type="checkbox"
            className="h-4 w-4"
            checked={cfg.upload_consent}
            onChange={(e) => patch({ upload_consent: e.target.checked })}
            aria-label={t('settings.tutorial.consent')}
          />
        </div>

        <div className="grid gap-1.5">
          <span className="text-sm font-medium">
            {t('settings.tutorial.auto_policy')}
          </span>
          <Select
            value={cfg.coach.auto_policy ?? 'off'}
            onValueChange={(v) => patchCoach({ auto_policy: v as 'off' | 'all' })}
          >
            <SelectTrigger className="h-9">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="off">
                {t('settings.tutorial.auto_policy_off')}
              </SelectItem>
              <SelectItem value="all">
                {t('settings.tutorial.auto_policy_all')}
              </SelectItem>
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            {t('settings.tutorial.auto_policy_hint')}
          </p>
        </div>

        <div className="grid gap-1.5">
          <span className="text-sm font-medium">
            {t('settings.tutorial.preset')}
          </span>
          <Select
            value={undefined}
            onValueChange={(id) => {
              const p = LLM_PRESETS.find((x) => x.id === id)
              if (p) patchLlm({ base_url: p.baseUrl, model: p.model })
            }}
          >
            <SelectTrigger className="h-9">
              <SelectValue placeholder={t('settings.tutorial.preset_choose')} />
            </SelectTrigger>
            <SelectContent>
              {LLM_PRESETS.map((p) => (
                <SelectItem key={p.id} value={p.id}>
                  {t(p.labelKey)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="grid gap-1.5">
          <span className="text-sm font-medium">
            {t('settings.tutorial.base_url')}
          </span>
          <Input
            value={cfg.llm.base_url}
            onChange={(e) => patchLlm({ base_url: e.target.value })}
            placeholder="https://open.bigmodel.cn/api/paas/v4"
          />
        </div>

        <div className="grid gap-1.5">
          <span className="text-sm font-medium">
            {t('settings.tutorial.model')}
          </span>
          <Input
            value={cfg.llm.model}
            onChange={(e) => patchLlm({ model: e.target.value })}
            placeholder="glm-4.7-flash"
          />
        </div>

        <div className="grid gap-1.5">
          <span className="text-sm font-medium">
            {t('settings.tutorial.api_key')}
          </span>
          <div className="flex gap-2">
            <Input
              type={showKey ? 'text' : 'password'}
              value={cfg.llm.api_key}
              onChange={(e) => patchLlm({ api_key: e.target.value })}
              placeholder={t('settings.tutorial.api_key_placeholder')}
              autoComplete="off"
            />
            <Button
              variant="outline"
              size="icon"
              className="h-9 w-9 shrink-0"
              onClick={() => setShowKey((v) => !v)}
              aria-label={t('settings.tutorial.api_key_toggle')}
            >
              {showKey ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">
            {t('settings.tutorial.api_key_hint')}
          </p>
        </div>
      </CardContent>
    </Card>
  )
}
