// Per-turn throttle for the commentary tile (D-0008 §2): at most one
// line per turn, keyed by `bakaze-kyoku-turn_count`; a new game resets
// both the key and the mute.

import type { GameStateSnapshot } from '@/types'

export function turnKey(snap: GameStateSnapshot): string {
  return `${snap.bakaze}-${snap.kyoku}-${snap.turn_count}`
}

export class CommentaryThrottle {
  private lastKey: string | null = null

  /** True exactly once per turn key. */
  allow(key: string): boolean {
    if (this.lastKey === key) return false
    this.lastKey = key
    return true
  }

  /** New game: forget the previous game's last turn. */
  reset(): void {
    this.lastKey = null
  }
}
