// The decision-window snapshot's tehai already contains the drawn tile
// (14 tiles at zero melds). ReplayBoard must drop one copy before encoding,
// so the hand row renders 13 tiles next to the `+` badge — never the drawn
// tile twice (which reads as holding a pair of it).

import { describe, expect, it, vi } from 'vitest'
import { render } from '@testing-library/react'

import { ReplayBoard } from './ReplayBoard'
import type { GameStateSnapshot, MahgenView } from '@/types'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}))

const OUR_SEAT = 1

const snap = (tehai: string[], drawn: string | null): GameStateSnapshot => ({
  bakaze: 'E',
  kyoku: 1,
  honba: 0,
  kyotaku: 0,
  oya: 0,
  current_player: OUR_SEAT,
  turn_count: 13,
  phase: 'wait_act',
  is_done: false,
  num_players: 4,
  players: Array.from({ length: 4 }, (_, seat) => ({
    seat,
    tehai: seat === OUR_SEAT ? tehai : [],
    melds: [],
    river: [],
    score: 25000,
    riichi_declared: false,
    riichi_stage: false,
    double_riichi: false,
    riichi_declaration_index: null,
    kita_tiles: [],
    drawn_tile: seat === OUR_SEAT ? drawn : null,
  })),
  dora_markers: [],
  our_seat: OUR_SEAT,
})

const view = (hand: string): MahgenView => ({
  players: [{ seat: OUR_SEAT, hand, melds: [], river: '' }],
  num_players: 4,
  dora_indicators: '',
})

// data-seq attributes of every rendered mahgen tile group, in DOM order.
const seqs = () =>
  Array.from(document.querySelectorAll('mah-gen[data-seq]')).map((el) =>
    el.getAttribute('data-seq'),
  )

describe('ReplayBoard drawn tile', () => {
  it('renders 13 tiles plus the badge when the drawn tile is new to the hand', () => {
    // Turn-7 shape from game 01M4N9NH…: 2s2s 4s 7s7s 8s8s C C F N N W + drawn 8m.
    render(
      <ReplayBoard
        view={view('8m2247788s344677z')}
        snapshot={snap(
          ['2s', '2s', '4s', '7s', '7s', '8s', '8s', 'C', 'C', 'F', 'N', 'N', 'W', '8m'],
          '8m',
        )}
        ourSeat={OUR_SEAT}
      />,
    )
    expect(seqs()).toEqual(['2247788s344677z', '8m'])
  })

  it('keeps the held copy when the drawn tile pairs one already in hand', () => {
    render(
      <ReplayBoard
        view={view('88m2247788s44677z')}
        snapshot={snap(
          ['2s', '2s', '4s', '7s', '7s', '8s', '8s', 'C', 'C', 'F', 'N', 'N', '8m', '8m'],
          '8m',
        )}
        ourSeat={OUR_SEAT}
      />,
    )
    expect(seqs()).toEqual(['8m2247788s44677z', '8m'])
  })

  it('falls back to the backend hand string when nothing was drawn', () => {
    render(
      <ReplayBoard
        view={view('2247788s344677z')}
        snapshot={snap(['2s', '2s', '4s', '7s', '7s', '8s', '8s', 'C', 'C', 'F', 'N', 'N', 'W'], null)}
        ourSeat={OUR_SEAT}
      />,
    )
    expect(seqs()).toEqual(['2247788s344677z'])
  })
})
