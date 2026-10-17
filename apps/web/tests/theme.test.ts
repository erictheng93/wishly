// @vitest-environment happy-dom
import { readFileSync } from 'node:fs'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'
import { useTheme } from '../app/composables/useTheme'

// 首繪前的內嵌腳本寫在 nuxt.config.ts（無法直接 import），從原始碼取出字串來執行
const inline = readFileSync('nuxt.config.ts', 'utf8').match(/innerHTML: "(try\{var t=localStorage[^"]+)"/)![1]!
const theme = () => document.documentElement.dataset.theme

function runInline(storage: () => string | null) {
  vi.stubGlobal('localStorage', { getItem: storage })
  new Function(inline)()
}

beforeEach(() => {
  delete document.documentElement.dataset.theme
  vi.unstubAllGlobals()
})

describe('內嵌腳本：首繪前的主題', () => {
  it('沒存過偏好 → light（即使系統是深色）', () => {
    runInline(() => null)
    expect(theme()).toBe('light')
  })
  it.each(['light', 'dark'])('存 %s → 照用', (v) => {
    runInline(() => v)
    expect(theme()).toBe(v)
  })
  it("存 'auto' → 不設 data-theme，交給 prefers-color-scheme", () => {
    runInline(() => 'auto')
    expect(theme()).toBeUndefined()
  })
  it('存了無法辨識的值 → light', () => {
    runInline(() => 'blue')
    expect(theme()).toBe('light')
  })
  it('localStorage 拋錯（無痕／封鎖儲存）→ light', () => {
    runInline(() => { throw new Error('blocked') })
    expect(theme()).toBe('light')
  })
})

describe('useTheme().toggle', () => {
  const store = new Map<string, string>()
  beforeEach(() => {
    store.clear()
    vi.stubGlobal('localStorage', { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) })
    vi.stubGlobal('useState', (_k: string, init: () => unknown) => ref(init()))
  })

  it('light → dark → auto → light 循環，並把選擇寫入 ws_theme', () => {
    document.documentElement.dataset.theme = 'light'
    const t = useTheme()
    t.toggle()
    expect([theme(), store.get('ws_theme'), t.mode.value]).toEqual(['dark', 'dark', 'dark'])
    t.toggle()
    expect([theme(), store.get('ws_theme'), t.mode.value]).toEqual([undefined, 'auto', 'auto'])
    t.toggle()
    expect([theme(), store.get('ws_theme'), t.mode.value]).toEqual(['light', 'light', 'light'])
  })

  it("選「自動」後重新載入仍是自動：存 'auto' 而非移除，內嵌腳本不會改回 light", () => {
    document.documentElement.dataset.theme = 'dark'
    useTheme().toggle()
    expect(store.get('ws_theme')).toBe('auto')
    delete document.documentElement.dataset.theme
    runInline(() => store.get('ws_theme') ?? null)
    expect(theme()).toBeUndefined()
  })

  it('localStorage 寫入失敗時仍套用當次', () => {
    vi.stubGlobal('localStorage', { setItem: () => { throw new Error('blocked') } })
    document.documentElement.dataset.theme = 'light'
    useTheme().toggle()
    expect(theme()).toBe('dark')
  })
})
