// Tutorial route: `?game=<ulid>` opens the AI replay/coaching view for
// that recorded game; without a param it shows a picker over the
// existing history cache. The heavy lifting lives in `Replay.tsx` and
// `stores/tutorialStore`.

import { useEffect } from 'react'
import { useSearchParams } from 'react-router-dom'
import { useTranslation } from 'react-i18next'
import { GraduationCap } from 'lucide-react'

import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ScrollArea } from '@/components/ui/scroll-area'
import { invoke } from '@/lib/tauri'
import { cn } from '@/lib/utils'
import { useHistoryStore } from '@/stores/historyStore'
import type { GameRecord } from '@/types'
import { Replay } from './Replay'

export function Tutorial() {
  const [params, setParams] = useSearchParams()
  const gameId = params.get('game')

  if (gameId) {
    return <Replay key={gameId} gameId={gameId} onExit={() => setParams({})} />
  }
  return <GamePicker onPick={(id) => setParams({ game: id })} />
}

function GamePicker({ onPick }: { onPick: (id: string) => void }) {
  const { t } = useTranslation()
  const records = useHistoryStore((s) => s.records)
  const loading = useHistoryStore((s) => s.loading)
  const setRecords = useHistoryStore((s) => s.setRecords)
  const setLoading = useHistoryStore((s) => s.setLoading)

  useEffect(() => {
    let cancelled = false
    void (async () => {
      setLoading(true)
      try {
        const list = await invoke<GameRecord[]>('list_game_history', {
          filter: null,
          limit: 0,
          offset: 0,
        })
        if (!cancelled) setRecords(list)
      } catch {
        /* browser mode / load error — the empty state explains itself */
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [setRecords, setLoading])

  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          {t('tutorial.title')}
        </h1>
        <p className="text-sm text-muted-foreground">{t('tutorial.subtitle')}</p>
      </div>
      <Card>
        <CardHeader>
          <CardTitle>{t('tutorial.pick_game')}</CardTitle>
        </CardHeader>
        <CardContent>
          {records.length === 0 ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              {loading ? t('tutorial.loading') : t('tutorial.empty')}
            </p>
          ) : (
            <ScrollArea className="max-h-[28rem]">
              <ul className="space-y-1 pr-2">
                {records.map((r) => (
                  <li key={r.id}>
                    <button
                      type="button"
                      onClick={() => onPick(r.id)}
                      className={cn(
                        'flex w-full items-center gap-3 rounded-md border border-transparent px-3 py-2 text-left',
                        'hover:border-border hover:bg-accent/40',
                      )}
                    >
                      <GraduationCap className="h-4 w-4 shrink-0 text-muted-foreground" />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm">
                          {new Date(r.started_at).toLocaleString()}
                        </span>
                        <span className="block font-mono text-[11px] text-muted-foreground">
                          {r.id}
                        </span>
                      </span>
                      <Badge variant="outline">{r.num_players}p</Badge>
                      {r.our_rank != null && <Badge variant="secondary">#{r.our_rank}</Badge>}
                    </button>
                  </li>
                ))}
              </ul>
            </ScrollArea>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
