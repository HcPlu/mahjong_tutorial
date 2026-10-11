// The AI replay page: step through a recorded game event by event, see
// the board + engine candidates at every decision, and read the LLM
// coaching text (generated on demand, cached per game). Numbers-only
// replay works with zero configuration — the LLM is an optional layer.

import { useEffect, useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router-dom'
import {
  Bot,
  ChevronLeft,
  ChevronRight,
  Loader2,
  RefreshCw,
  Sparkles,
  Star,
} from 'lucide-react'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { cn } from '@/lib/utils'
import { CandidateChart } from '@/components/tutorial/CandidateChart'
import { ExplanationCard } from '@/components/tutorial/ExplanationCard'
import { ReplayBoard } from '@/components/tutorial/ReplayBoard'
import { getTutorialConfig, outboundAllowed } from '@/components/tutorial/configGate'
import { GenerationParams } from '@/components/tutorial/GenerationParams'
import { tileNamesZh } from '@/components/tutorial/tileText'
import type { OutlineDecision } from '@/components/tutorial/types'
import { useConfigStore } from '@/stores/configStore'
import { useTutorialStore } from '@/stores/tutorialStore'

type Props = {
  gameId: string
  onExit: () => void
}

export function Replay({ gameId, onExit }: Props) {
  const { t } = useTranslation()
  const st = useTutorialStore()
  const config = useConfigStore((s) => s.config)
  const tutorialCfg = getTutorialConfig(config)
  const canOutbound = outboundAllowed(tutorialCfg)

  useEffect(() => {
    void st.openGame(gameId).then(() => {
      // Bot column without a click: the reference replay is local, fast
      // and cached forever — generate it on open whenever a replayable
      // bot is configured. The manual button stays for retry after an
      // error. (Read via getState: config identity churn must not reset
      // the stepper by re-running this effect.)
      const s = useTutorialStore.getState()
      const cfg = useConfigStore.getState().config
      const active =
        s.outline?.num_players === 3 ? cfg?.bot.active_3p : cfg?.bot.active_4p
      if (!s.botRef && !s.botRefError && active) void s.loadBotRef(true)
    })
    // openGame is stable enough for this page's lifetime (keyed by gameId
    // at the route level); re-running on store identity churn would reset
    // the stepper mid-session.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gameId])

  const outline = st.outline
  const step = st.step
  const snap = step?.snapshot ?? null

  // The decision whose draw window contains the current step (its Tsumo
  // already played, the next decision's not yet).
  const currentDecision = useMemo<OutlineDecision | null>(() => {
    if (!outline) return null
    let cur: OutlineDecision | null = null
    for (const d of outline.decisions) {
      if (d.event_index <= st.stepIndex) cur = d
      else break
    }
    return cur
  }, [outline, st.stepIndex])

  const atDecisionWindow = step?.analysis != null
  const turnDoc =
    atDecisionWindow && currentDecision
      ? st.doc?.turns.find((x) => x.junme === currentDecision.junme) ?? null
      : null

  const keyCount = outline?.decisions.filter((d) => d.is_key).length ?? 0

  return (
    <div className="space-y-4">
      {/* header */}
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="ghost" size="sm" onClick={onExit}>
          <ChevronLeft className="h-4 w-4" />
          {t('tutorial.back')}
        </Button>
        <h1 className="text-lg font-semibold tracking-tight">
          {t('tutorial.replay_title')}
        </h1>
        <span className="font-mono text-xs text-muted-foreground">{gameId}</span>
        {snap && (
          <Badge variant="outline">
            {snap.bakaze}
            {snap.kyoku}
            {snap.honba > 0 ? `-${snap.honba}` : ''}
          </Badge>
        )}
      </div>

      {/* overview + generation controls */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0">
          <CardTitle className="text-base">{t('tutorial.overview')}</CardTitle>
          <div className="flex items-center gap-2">
            <Select
              value={st.coverage}
              onValueChange={(v) => st.setCoverage(v as 'key' | 'all')}
            >
              <SelectTrigger className="h-8 w-40 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="key">
                  {t('tutorial.coverage_key', { n: keyCount })}
                </SelectItem>
                <SelectItem value="all">
                  {t('tutorial.coverage_all', {
                    n: outline?.decisions.length ?? 0,
                  })}
                </SelectItem>
              </SelectContent>
            </Select>
            <Button
              size="sm"
              variant="outline"
              disabled={st.botRefLoading}
              onClick={() => void st.loadBotRef()}
              title={t('tutorial.bot_ref_hint')}
            >
              {st.botRefLoading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Bot className="h-3.5 w-3.5" />
              )}
              {st.botRef
                ? t('tutorial.bot_ref_done', { bot: st.botRef.bot })
                : t('tutorial.bot_ref')}
            </Button>
            {st.doc ? (
              <Button
                size="sm"
                variant="outline"
                disabled={st.generating || !canOutbound}
                onClick={() => void st.generate()}
              >
                {st.generating ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <RefreshCw className="h-3.5 w-3.5" />
                )}
                {t('tutorial.regenerate')}
              </Button>
            ) : (
              <Button
                size="sm"
                disabled={st.generating || !canOutbound}
                onClick={() => void st.generate()}
              >
                {st.generating ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Sparkles className="h-3.5 w-3.5" />
                )}
                {t('tutorial.generate')}
              </Button>
            )}
          </div>
        </CardHeader>
        <CardContent className="space-y-2">
          <GenerationParams />
          {st.doc?.overview ? (
            <p className="text-sm leading-relaxed">{tileNamesZh(st.doc.overview)}</p>
          ) : st.generating ? (
            <p className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t('tutorial.generating')}
            </p>
          ) : canOutbound ? (
            <p className="text-sm text-muted-foreground">
              {t('tutorial.generate_hint')}
            </p>
          ) : (
            <p className="text-sm text-muted-foreground">
              {t('tutorial.not_configured')}{' '}
              <Link className="underline underline-offset-2" to="/settings">
                {t('tutorial.go_settings')}
              </Link>
              {t('tutorial.numbers_only_hint')}
            </p>
          )}
          {st.doc?.model && (
            <p className="font-mono text-[11px] text-muted-foreground">
              {st.doc.model} · {new Date(st.doc.created_at).toLocaleString()}
            </p>
          )}
          {st.generateError && (
            <p className="text-sm text-red-500">{st.generateError}</p>
          )}
          {st.botRefError && (
            <p className="text-sm text-red-500">{st.botRefError}</p>
          )}
        </CardContent>
      </Card>

      {/* stepper */}
      {outline && (
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="icon"
              className="h-8 w-8"
              disabled={st.stepIndex <= 0}
              onClick={() => void st.stepBy(-1)}
              aria-label={t('tutorial.step_prev')}
            >
              <ChevronLeft className="h-4 w-4" />
            </Button>
            <input
              type="range"
              min={0}
              max={Math.max(0, outline.total_events - 1)}
              value={st.stepIndex}
              onChange={(e) => void st.setStepIndex(Number(e.target.value))}
              className="h-8 flex-1"
              aria-label={t('tutorial.event_slider')}
            />
            <Button
              variant="outline"
              size="icon"
              className="h-8 w-8"
              disabled={st.stepIndex >= outline.total_events - 1}
              onClick={() => void st.stepBy(1)}
              aria-label={t('tutorial.step_next')}
            >
              <ChevronRight className="h-4 w-4" />
            </Button>
            <span className="w-20 shrink-0 text-right font-mono text-xs text-muted-foreground">
              {st.stepIndex + 1}/{outline.total_events}
            </span>
          </div>
          <div className="flex flex-wrap items-center gap-1">
            {outline.decisions.map((d) => (
              <button
                key={d.junme}
                type="button"
                onClick={() => void st.jumpToDecision(d.junme)}
                className={cn(
                  'rounded border px-1.5 py-0.5 font-mono text-[11px]',
                  d.event_index === st.stepIndex
                    ? 'border-primary bg-primary/10 text-primary'
                    : 'border-border text-muted-foreground hover:bg-accent/40',
                )}
                title={
                  d.is_key ? t('tutorial.key_turn') : t('tutorial.turn_n', { n: d.junme })
                }
              >
                {d.junme}
                {d.is_key && (
                  <Star className="ml-0.5 inline h-2.5 w-2.5 text-amber-500" />
                )}
                {st.doc?.turns.some((x) => x.junme === d.junme) && (
                  <span
                    className="ml-0.5 inline-block h-1.5 w-1.5 rounded-full bg-emerald-500 align-middle"
                    title={t('tutorial.has_explanation')}
                  />
                )}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* board + analysis */}
      {snap && step && st.outline && (
        <div className="grid gap-4 lg:grid-cols-2">
          <ReplayBoard view={step.mahgen} snapshot={snap} ourSeat={st.outline.seat} />
          <div className="space-y-4">
            {step.analysis ? (
              <Card>
                <CardHeader className="pb-2">
                  <CardTitle className="flex items-center gap-2 text-base">
                    {t('tutorial.candidates')}
                    {currentDecision && (
                      <span className="text-xs font-normal text-muted-foreground">
                        {t('tutorial.turn_n', { n: currentDecision.junme })}
                      </span>
                    )}
                  </CardTitle>
                </CardHeader>
                <CardContent>
                  {st.liveBot && (
                    <p className="mb-2 text-[11px] text-muted-foreground">
                      {t('tutorial.live_bot_note', { bot: st.liveBot.bot })}
                    </p>
                  )}
                  <CandidateChart
                    analysis={step.analysis}
                    actual={currentDecision?.actual ?? null}
                    botTurn={
                      currentDecision
                        ? st.botRef?.turns.find((x) => x.junme === currentDecision.junme) ?? null
                        : null
                    }
                    liveTurn={
                      currentDecision
                        ? st.liveBot?.turns.find((x) => x.junme === currentDecision.junme) ?? null
                        : null
                    }
                  />
                </CardContent>
              </Card>
            ) : (
              <Card>
                <CardContent className="py-6 text-center text-sm text-muted-foreground">
                  {t('tutorial.no_decision_here')}
                </CardContent>
              </Card>
            )}

            {atDecisionWindow && currentDecision && (
              <Card>
                <CardHeader className="pb-2">
                  <CardTitle className="text-base">
                    {t('tutorial.explanation')}
                  </CardTitle>
                </CardHeader>
                <CardContent>
                  {turnDoc ? (
                    <ExplanationCard doc={turnDoc} actual={currentDecision.actual} />
                  ) : st.generating ? null : (
                    <p className="text-sm text-muted-foreground">
                      {st.doc
                        ? t('tutorial.explanation_missing')
                        : t('tutorial.explanation_need_generate')}
                    </p>
                  )}
                </CardContent>
              </Card>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
