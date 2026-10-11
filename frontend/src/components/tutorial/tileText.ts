// Post-processing for LLM explanation text: rewrite mjai tile codes
// (1p, 7s, E, 5mr, 0m …) into Chinese tile names (一筒, 七索, 东, 红五万 …).
//
// Display-only sugar for the translated view — the raw text stays one
// toggle away, and nothing here touches what gets stored or validated.
//
// Boundary rules: a code only matches when it is not glued to another
// ASCII alphanumeric, so `p=0.42`, `max_tokens` or "AI" never transform;
// Chinese characters around a code are fine (they are non-word chars
// for \b purposes).

const NUMERALS = ['一', '二', '三', '四', '五', '六', '七', '八', '九'] as const
const SUITS: Record<string, string> = { m: '万', p: '筒', s: '索' }
const HONORS: Record<string, string> = {
  E: '东',
  S: '南',
  W: '西',
  N: '北',
  P: '白',
  F: '发',
  C: '中',
}

// 5mr / 5pr / 5sr → 红五万 …
const RED_SUFFIX_RE = /(?<![0-9A-Za-z])5([mps])r(?![0-9A-Za-z])/g
// 0m / 0p / 0s → 红五万 … (Majsoul-style zero notation)
const RED_ZERO_RE = /(?<![0-9A-Za-z])0([mps])(?![0-9A-Za-z])/g
// A maximal run of digit+suit codes (1p2p3p …) — the alternation makes
// glued non-tokens like "12m" or "3ms" fail the boundaries and stay raw.
const RUN_RE = /(?<![0-9A-Za-z])[1-9][mps](?:[1-9][mps])*(?![0-9A-Za-z])/g
// Standalone honor letters → 东 南 西 北 白 发 中
const HONOR_RE = /\b([ESWNPFC])\b/g

/** Rewrite every tile code in `text` to its Chinese tile name. */
export function tileNamesZh(text: string): string {
  return text
    .replace(RED_SUFFIX_RE, (_m, suit: string) => `红五${SUITS[suit]}`)
    .replace(RED_ZERO_RE, (_m, suit: string) => `红五${SUITS[suit]}`)
    .replace(RUN_RE, (run) =>
      run.replace(/([1-9])([mps])/g, (_p, n: string, suit: string) => `${NUMERALS[Number(n) - 1]}${SUITS[suit]}`),
    )
    .replace(HONOR_RE, (_m, h: string) => HONORS[h])
}
