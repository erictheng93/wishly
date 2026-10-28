import { describe, expect, it } from 'vitest'
import { code39 } from '../app/utils/code39'

// 把條 / 空寬度還原成 n/w 字串（條空交錯）
const widths = (v: string) => {
  const { bars, width } = code39(v)
  const out: number[] = []; let x = 0
  for (const b of bars) { if (b.x > x) out.push(b.x - x); out.push(b.w); x = b.x + b.w }
  return { out, width }
}
describe('code39', () => {
  it('起訖 * = nwnnwnwnn（窄寬窄窄寬窄寬窄窄）', () => {
    const { out } = widths('')
    expect(out.slice(0, 9).map(w => (w === 3 ? 'w' : 'n')).join('')).toBe('nwnnwnwnn')
  })
  it('數字 1 = wnnwnnnnw；總寬 = 3 字 × 15 + 2 字間隔', () => {
    const { out, width } = widths('1')
    expect(out.slice(10, 19).map(w => (w === 3 ? 'w' : 'n')).join('')).toBe('wnnwnnnnw') // 跳過第一字與 1 單位字間隔
    expect(width).toBe(3 * 15 + 2)
  })
  it('超商條碼字元可編；小寫、* 與其他符號丟錯', () => {
    expect(() => code39('1311053TW')).not.toThrow()
    expect(() => code39('abc')).toThrow()
    expect(() => code39('A*B')).toThrow()
    expect(() => code39('A@')).toThrow()
  })
})
