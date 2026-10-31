import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { trackShareView } from '../app/utils/shareView'
import { L } from '../app/utils/analytics'

const store = new Map<string, string>()
const fetchMock = vi.fn()
beforeEach(() => {
  store.clear(); fetchMock.mockReset(); fetchMock.mockResolvedValue({})
  vi.stubGlobal('fetch', fetchMock)
  vi.stubGlobal('localStorage', { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) })
  vi.stubGlobal('document', { referrer: 'https://l.facebook.com/x' })
  vi.stubGlobal('location', { hostname: 'wishly.tw' })
})
afterEach(() => { vi.unstubAllGlobals() })

const body = () => JSON.parse(fetchMock.mock.calls[0][1].body)

describe('trackShareView', () => {
  it('sends once with visitor id and referrer host, no credentials', () => {
    trackShareView('https://api', 'abcdefghij', false)
    expect(fetchMock).toHaveBeenCalledTimes(1)
    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('https://api/api/v1/public/wishlists/abcdefghij/views')
    expect(init).toMatchObject({ method: 'POST', credentials: 'omit', keepalive: true })
    expect(body().referrer_host).toBe('l.facebook.com')
    expect(body().visitor_id).toBe(store.get('ws_vid'))
  })
  it('reuses stored visitor id and drops same-site referrer', () => {
    store.set('ws_vid', '11111111-1111-4111-8111-111111111111')
    vi.stubGlobal('document', { referrer: 'https://wishly.tw/' })
    trackShareView('https://api', 'abcdefghij', false)
    expect(body().visitor_id).toBe('11111111-1111-4111-8111-111111111111')
    expect(body().referrer_host).toBeUndefined()
  })
  it('skips logged-in, native and demo', () => {
    trackShareView('https://api', 'abcdefghij', true)
    trackShareView('https://api', 'demo', false)
    vi.stubGlobal('__NATIVE__', true)
    trackShareView('https://api', 'abcdefghij', false)
    expect(fetchMock).not.toHaveBeenCalled()
  })
  it('swallows rejected and thrown fetch', async () => {
    fetchMock.mockRejectedValueOnce(new Error('net'))
    expect(() => trackShareView('https://api', 'abcdefghij', false)).not.toThrow()
    fetchMock.mockImplementationOnce(() => { throw new Error('sync') })
    expect(() => trackShareView('https://api', 'abcdefghij', false)).not.toThrow()
    await new Promise(r => setTimeout(r, 0))
  })
  it('works when localStorage throws', () => {
    vi.stubGlobal('localStorage', { getItem: () => { throw new Error('denied') }, setItem: () => { throw new Error('denied') } })
    trackShareView('https://api', 'abcdefghij', false)
    expect(body().visitor_id).toBeUndefined()
  })
})

describe('analytics labels', () => {
  it('wishlistVisibility covers the four real visibilities', () => {
    const v = L.wishlistVisibility
    expect(Object.keys(v)).toEqual(['public', 'friends', 'selected', 'private'])
    expect(Object.values(v).every(x => !x.startsWith('creator.analytics'))).toBe(true)
    expect(L.device.mobile).not.toContain('creator.analytics')
  })
})
