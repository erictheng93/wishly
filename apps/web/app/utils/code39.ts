// Code 39 編碼（綠界超商條碼要出示三段可掃描條碼）。每字 9 個元素（條空交錯、條起頭），w=寬 n=窄，寬:窄 = 3:1，字間 1 單位窄空白
const P: Record<string, string> = {
  '0': 'nnnwwnwnn', '1': 'wnnwnnnnw', '2': 'nnwwnnnnw', '3': 'wnwwnnnnn', '4': 'nnnwwnnnw', '5': 'wnnwwnnnn', '6': 'nnwwwnnnn', '7': 'nnnwnnwnw', '8': 'wnnwnnwnn', '9': 'nnwwnnwnn',
  A: 'wnnnnwnnw', B: 'nnwnnwnnw', C: 'wnwnnwnnn', D: 'nnnnwwnnw', E: 'wnnnwwnnn', F: 'nnwnwwnnn', G: 'nnnnnwwnw', H: 'wnnnnwwnn', I: 'nnwnnwwnn', J: 'nnnnwwwnn',
  K: 'wnnnnnnww', L: 'nnwnnnnww', M: 'wnwnnnnwn', N: 'nnnnwnnww', O: 'wnnnwnnwn', P: 'nnwnwnnwn', Q: 'nnnnnnwww', R: 'wnnnnnwwn', S: 'nnwnnnwwn', T: 'nnnnwnwwn',
  U: 'wwnnnnnnw', V: 'nwwnnnnnw', W: 'wwwnnnnnn', X: 'nwnnwnnnw', Y: 'wwnnwnnnn', Z: 'nwwnwnnnn',
  '-': 'nwnnnnwnw', '.': 'wwnnnnwnn', ' ': 'nwwnnnwnn', '*': 'nwnnwnwnn', '$': 'nwnwnwnnn', '/': 'nwnwnnnwn', '+': 'nwnnnwnwn', '%': 'nnnwnwnwn',
}
export const CODE39_CHARSET = /^[0-9A-Z\-. $/+%]*$/

/** 回傳黑條 { x, w }（單位寬）與總寬；含起訖 `*`，不含 quiet zone。不合法字元（含小寫、`*`）丟錯 */
export function code39(value: string): { bars: { x: number, w: number }[], width: number } {
  if (!CODE39_CHARSET.test(value)) throw new Error('code39: invalid character')
  const bars: { x: number, w: number }[] = []
  let x = 0
  for (const ch of `*${value}*`) {
    [...P[ch]!].forEach((e, i) => {
      const w = e === 'w' ? 3 : 1
      if (i % 2 === 0) bars.push({ x, w })
      x += w
    })
    x += 1
  }
  return { bars, width: x - 1 }
}
