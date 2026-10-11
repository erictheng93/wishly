import { afterEach, describe, expect, it, vi } from 'vitest'
import { fetchListWithCreds } from '../app/utils/visibility'

const err = (status: number) => Object.assign(new Error(String(status)), { statusCode: status })

describe('fetchListWithCreds', () => {
  afterEach(() => vi.unstubAllGlobals())
  it('先打 /restricted，帶憑證與存取權杖', async () => {
    const f = vi.fn().mockResolvedValue({ slug: 'a' })
    vi.stubGlobal('$fetch', f)
    expect(await fetchListWithCreds('http://api', 'a', 'tok')).toEqual({ slug: 'a' })
    expect(f).toHaveBeenCalledTimes(1)
    expect(f.mock.calls[0]![0]).toBe('http://api/api/v1/restricted/wishlists/a')
    expect(f.mock.calls[0]![1]).toMatchObject({ credentials: 'include', headers: { 'X-List-Access': 'tok' } })
  })
  it('404（舊後端沒有 /restricted）→ 退回 /public 帶憑證', async () => {
    const f = vi.fn().mockRejectedValueOnce(err(404)).mockResolvedValueOnce({ slug: 'b' })
    vi.stubGlobal('$fetch', f)
    expect(await fetchListWithCreds('http://api', 'b', null)).toEqual({ slug: 'b' })
    expect(f.mock.calls[1]![0]).toBe('http://api/api/v1/public/wishlists/b')
    expect(f.mock.calls[1]![1]).toMatchObject({ credentials: 'include' })
  })
  it('403 等其他錯誤不退回（門檻頁要用這個結果）', async () => {
    const f = vi.fn().mockRejectedValue(err(403))
    vi.stubGlobal('$fetch', f)
    await expect(fetchListWithCreds('http://api', 'c', null)).rejects.toMatchObject({ statusCode: 403 })
    expect(f).toHaveBeenCalledTimes(1)
  })
})
