import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))

const store = new Map<string, string>()
vi.stubGlobal('localStorage', { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v), removeItem: (k: string) => void store.delete(k) })

// 模組層級變數：每個測試重新載入
const load = async () => { vi.resetModules(); return await import('../app/utils/native') }
const flush = () => new Promise(r => setTimeout(r, 0))

beforeEach(() => { invoke.mockReset(); invoke.mockResolvedValue(null) })
afterEach(() => { store.clear(); vi.stubGlobal('__NATIVE__', false) })

describe('網頁版', () => {
  it('不帶 header、不呼叫 keychain、不碰 localStorage', async () => {
    const n = await load()
    store.set('ws_session_token', 'x')
    await n.initNativeSession()
    n.setNativeToken('t')
    await flush()
    expect(n.nativeHeaders()).toEqual({})
    expect(invoke).not.toHaveBeenCalled()
    expect(store.get('ws_session_token')).toBe('x')
  })
})

describe('原生殼', () => {
  beforeEach(() => { vi.stubGlobal('__NATIVE__', true) })

  it('init 從 keychain 載入權杖', async () => {
    invoke.mockResolvedValue('abc')
    const n = await load()
    expect(n.nativeHeaders()).toEqual({ 'X-Native-Client': '1' })
    await n.initNativeSession()
    expect(invoke).toHaveBeenCalledWith('session_token_get')
    expect(n.nativeHeaders()).toEqual({ 'X-Native-Client': '1', Authorization: 'Bearer abc' })
  })

  it('舊版 localStorage 權杖：keychain 為空則搬入並刪除', async () => {
    store.set('ws_session_token', 'old')
    const n = await load()
    await n.initNativeSession()
    expect(invoke).toHaveBeenCalledWith('session_token_set', { token: 'old' })
    expect(store.has('ws_session_token')).toBe(false)
    expect(n.nativeHeaders().Authorization).toBe('Bearer old')
  })

  it('keychain 已有權杖：以 keychain 為準，仍刪除舊鍵', async () => {
    invoke.mockResolvedValue('new')
    store.set('ws_session_token', 'old')
    const n = await load()
    await n.initNativeSession()
    expect(invoke).not.toHaveBeenCalledWith('session_token_set', expect.anything())
    expect(store.has('ws_session_token')).toBe(false)
    expect(n.nativeHeaders().Authorization).toBe('Bearer new')
  })

  it('set / clear 更新記憶體並寫入 keychain，不寫 localStorage', async () => {
    const n = await load()
    n.setNativeToken('abc')
    expect(n.nativeHeaders().Authorization).toBe('Bearer abc')
    await flush()
    expect(invoke).toHaveBeenCalledWith('session_token_set', { token: 'abc' })
    n.setNativeToken(null)
    expect(n.nativeHeaders()).toEqual({ 'X-Native-Client': '1' })
    await flush()
    expect(invoke).toHaveBeenCalledWith('session_token_clear')
    expect(store.size).toBe(0)
  })

  it('keychain 失敗只警告且不含權杖', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    invoke.mockRejectedValue(new Error('boom'))
    const n = await load()
    n.setNativeToken('secret')
    await flush()
    expect(n.nativeHeaders().Authorization).toBe('Bearer secret')
    expect(JSON.stringify(warn.mock.calls)).not.toContain('secret')
    warn.mockRestore()
  })
})
