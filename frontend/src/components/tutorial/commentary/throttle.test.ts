import { describe, expect, it } from 'vitest'

import { CommentaryThrottle, turnKey } from './throttle'
import type { GameStateSnapshot } from '@/types'

const snap = (over: Partial<GameStateSnapshot>) =>
  ({ bakaze: 'E', kyoku: 1, turn_count: 0, ...over }) as GameStateSnapshot

describe('commentary throttle', () => {
  it('keys turns by round + kyoku + turn count', () => {
    expect(turnKey(snap({ bakaze: 'E', kyoku: 2, turn_count: 7 }))).toBe('E-2-7')
    expect(turnKey(snap({ bakaze: 'S', kyoku: 1, turn_count: 3 }))).toBe('S-1-3')
  })

  it('allows exactly one line per turn key', () => {
    const t = new CommentaryThrottle()
    expect(t.allow('E-1-3')).toBe(true)
    expect(t.allow('E-1-3')).toBe(false)
    expect(t.allow('E-1-4')).toBe(true)
  })

  it('reset lets the same key speak again (new game)', () => {
    const t = new CommentaryThrottle()
    expect(t.allow('E-1-3')).toBe(true)
    t.reset()
    expect(t.allow('E-1-3')).toBe(true)
  })
})
