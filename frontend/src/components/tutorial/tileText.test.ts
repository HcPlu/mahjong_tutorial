import { describe, expect, it } from 'vitest'

import { tileNamesZh } from './tileText'

describe('tileNamesZh', () => {
  it('translates number tiles with Chinese neighbours', () => {
    expect(tileNamesZh('切5s后听牌')).toBe('切五索后听牌')
    expect(tileNamesZh('保留1p2p3p的顺子')).toBe('保留一筒二筒三筒的顺子')
    expect(tileNamesZh('打7m比9p好')).toBe('打七万比九筒好')
  })

  it('translates honor letters and red fives', () => {
    expect(tileNamesZh('E是场风兼自风')).toBe('东是场风兼自风')
    expect(tileNamesZh('先拆N或P')).toBe('先拆北或白')
    expect(tileNamesZh('摸到5mr')).toBe('摸到红五万')
    expect(tileNamesZh('0s当红五看')).toBe('红五索当红五看')
  })

  it('leaves non-tile tokens untouched', () => {
    const keep = 'p=0.42 和 max_tokens 以及 AI 明确推荐 (42%)'
    expect(tileNamesZh(keep)).toBe(keep)
    // glued codes are not tiles: "12m" (two digits) stays, "3ms" stays
    expect(tileNamesZh('12m 3ms')).toBe('12m 3ms')
  })

  it('full sentence round-trip readability', () => {
    const src = '实际切P可以接受，切4m更优：4m铳率0.0最低，切7s则达0.023。'
    const out = tileNamesZh(src)
    expect(out).toContain('实际切白可以接受')
    expect(out).toContain('切四万更优')
    expect(out).toContain('七索')
  })
})
