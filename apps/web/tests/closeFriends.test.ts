import { beforeEach, expect, it, vi } from 'vitest'
import { useCloseFriends } from '../app/composables/useCloseFriends'

const api = vi.fn()
beforeEach(() => {
  const st = new Map<string, any>()
  vi.stubGlobal('useState', (k: string, init: () => any) => { if (!st.has(k)) st.set(k, { value: init() }); return st.get(k) })
  vi.stubGlobal('useApi', () => ({ api }))
  api.mockReset()
})

it('toggle on PUTs and adds, off DELETEs and removes', async () => {
  api.mockResolvedValue(undefined)
  const c = useCloseFriends()
  await c.toggle('u1', true)
  expect(api).toHaveBeenCalledWith('/me/close-friends/u1', { method: 'PUT' })
  expect(c.ids.value.has('u1')).toBe(true)
  await c.toggle('u1', false)
  expect(api).toHaveBeenLastCalledWith('/me/close-friends/u1', { method: 'DELETE' })
  expect(c.ids.value.has('u1')).toBe(false)
})

it('restores ids on failure', async () => {
  const c = useCloseFriends()
  api.mockRejectedValueOnce(new Error('x'))
  await expect(c.toggle('u1', true)).rejects.toThrow()
  expect(c.ids.value.has('u1')).toBe(false)
})

it('load fills list and ids', async () => {
  api.mockResolvedValue({ users: [{ id: 'a' }, { id: 'b' }] })
  const c = useCloseFriends()
  await c.load()
  expect([...c.ids.value]).toEqual(['a', 'b'])
})
