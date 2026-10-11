// Replay board for the tutorial page: our seat prominent at the bottom,
// opponents compact on top. Data comes from the replay stepper (a
// MahgenView derived server-side from a historical snapshot) — not from
// the live game store, so the live board tile stays untouched.

import { useTranslation } from 'react-i18next'

import { Mahgen } from '@/components/Mahgen'
import { mjaiToMahgen } from '@/lib/tileIdx'
import { Badge } from '@/components/ui/badge'
import type { GameStateSnapshot, MahgenView, PlayerMahgenView } from '@/types'

const WINDS = ['E', 'S', 'W', 'N'] as const

type Props = {
  view: MahgenView
  snapshot: GameStateSnapshot
  ourSeat: number
}

export function ReplayBoard({ view, snapshot, ourSeat }: Props) {
  const { t } = useTranslation()
  const opponents = view.players.filter((p) => p.seat !== ourSeat)
  const me = view.players.find((p) => p.seat === ourSeat)
  const meSnap = snapshot.players[ourSeat]
  const drawn = meSnap?.drawn_tile ?? null
  // The decision-window tehai already holds the drawn tile (14 tiles at
  // zero melds). Drop one copy before encoding so the row shows 13 tiles
  // next to the `+` badge instead of the drawn tile twice.
  const fallbackHand = me?.hand ?? ''
  const handSeq = (() => {
    if (!meSnap || !drawn) return fallbackHand
    const i = meSnap.tehai.indexOf(drawn)
    if (i < 0) return fallbackHand
    return mjaiToMahgen(meSnap.tehai.filter((_, k) => k !== i))
  })()

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-start gap-3">
        {opponents.map((p) => (
          <OpponentRow key={p.seat} player={p} snapshot={snapshot} />
        ))}
      </div>

      {me && (
        <div className="space-y-2 rounded-lg border border-border bg-card p-3">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="secondary">{t('tutorial.you')}</Badge>
            <span className="font-mono text-xs text-muted-foreground">
              {meSnap ? meSnap.score.toLocaleString() : ''}
            </span>
            <div className="ml-auto flex items-center gap-1">
              <span className="text-xs text-muted-foreground">
                {t('tutorial.dora')}
              </span>
              {view.dora_indicators && (
                <Mahgen seq={view.dora_indicators} kind="dora" />
              )}
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Mahgen seq={handSeq} kind="board-hand" />
            {drawn && (
              <span className="mx-1 text-muted-foreground">+</span>
            )}
            {drawn && <Mahgen seq={mjaiToMahgen([drawn])} kind="board-hand" />}
          </div>
          {me.melds.length > 0 && (
            <div className="flex flex-wrap items-center gap-2">
              {me.melds.map((m, i) => (
                <Mahgen key={i} seq={m} kind="board-meld" />
              ))}
            </div>
          )}
          <div>
            <Mahgen seq={me.river} kind="board-river" riverMode />
          </div>
        </div>
      )}
    </div>
  )
}

function OpponentRow({
  player,
  snapshot,
}: {
  player: PlayerMahgenView
  snapshot: GameStateSnapshot
}) {
  const { t } = useTranslation()
  const snap = snapshot.players[player.seat]
  const wind = WINDS[(player.seat + snapshot.num_players - snapshot.oya) % snapshot.num_players]
  const riichi = snap?.riichi_declared ?? false

  return (
    <div className="min-w-[12rem] flex-1 space-y-1 rounded-lg border border-border p-2">
      <div className="flex items-center gap-2">
        <span className="text-xs font-medium text-muted-foreground">
          {t('tutorial.opponent', { wind, seat: player.seat })}
        </span>
        {riichi && (
          <Badge variant="outline" className="h-4 px-1 text-[10px] text-red-500">
            {t('tutorial.riichi_badge')}
          </Badge>
        )}
        <span className="ml-auto font-mono text-xs text-muted-foreground">
          {snap ? snap.score.toLocaleString() : ''}
        </span>
      </div>
      {player.melds.length > 0 && (
        <div className="flex flex-wrap items-center gap-1">
          {player.melds.map((m, i) => (
            <Mahgen key={i} seq={m} kind="board-meld" />
          ))}
        </div>
      )}
      <Mahgen seq={player.river} kind="board-river" riverMode />
    </div>
  )
}
