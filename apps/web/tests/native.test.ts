import { afterEach, describe, expect, it, vi } from 'vitest'
import { nativeHeaders, setNativeToken } from '../app/utils/native'

const store = new Map<string, string>()
vi.stubGlobal('localStorage', { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v), removeItem: (k: string) => void store.delete(k) })
afterEach(() => { store.clear(); vi.stubGlobal('__NATIVE__', false) })

describe('原生殼身分', () => {
  it('網頁版：不帶任何 header，也不存權杖', () => {
    setNativeToken('t')
    expect(nativeHeaders()).toEqual({})
    expect(store.size).toBe(0)
  })
  it('原生殼：登入前只標記 X-Native-Client，登入後帶 Bearer，登出後清除', () => {
    vi.stubGlobal('__NATIVE__', true)
    expect(nativeHeaders()).toEqual({ 'X-Native-Client': '1' })
    setNativeToken('abc')
    expect(nativeHeaders()).toEqual({ 'X-Native-Client': '1', Authorization: 'Bearer abc' })
    setNativeToken(null)
    expect(nativeHeaders()).toEqual({ 'X-Native-Client': '1' })
  })
})
