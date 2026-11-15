import { expect, it } from 'vitest'
import { claimVisNote, needsCredsFetch } from '../app/utils/claimerVis'

const anonItem = { claimers: [{ display_name: 'a', anonymous: true }] }
const base = { loggedIn: true, visibility: 'public', claimersVisible: true, items: [anonItem] }
it('needsCredsFetch only when logged in, public, names on and something anonymous', () => {
  expect(needsCredsFetch(base)).toBe(true)
  expect(needsCredsFetch({ ...base, items: [{ contributors: [{ anonymous: true }] }] })).toBe(true)
  expect(needsCredsFetch({ ...base, loggedIn: false })).toBe(false)
  expect(needsCredsFetch({ ...base, visibility: 'friends' })).toBe(false)
  expect(needsCredsFetch({ ...base, claimersVisible: false })).toBe(false)
  expect(needsCredsFetch({ ...base, items: [{ claimers: [{ anonymous: false }] }, {}] })).toBe(false)
  expect(needsCredsFetch({ ...base, items: undefined })).toBe(false)
})
it('claimVisNote follows the copy matrix', () => {
  expect(claimVisNote({ anon: true, surpriseLocked: true, vis: 'public' })).toBe('guest.visWho.anon')
  expect(claimVisNote({ anon: false, surpriseLocked: true, vis: 'public' })).toBe('guest.visWho.surprise')
  expect(claimVisNote({ anon: false, claimersVisible: false, vis: 'public' })).toBe('guest.visWho.hidden')
  for (const v of ['public', 'friends', 'private']) expect(claimVisNote({ anon: false, claimersVisible: true, vis: v })).toBe(`guest.visWho.${v}`)
  expect(claimVisNote({ anon: false, claimersVisible: true })).toBeNull()
})
