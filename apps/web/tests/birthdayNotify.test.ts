import { describe, expect, it } from 'vitest'
import { birthdayNotifyBlock } from '../app/utils/birthdayNotify'

describe('birthdayNotifyBlock', () => {
  const ok = { birthday: '05-01', birthday_visibility: 'friends' }
  it('allows when birthday set, visible, list not private', () => {
    expect(birthdayNotifyBlock(ok, 'friends')).toBeNull()
    expect(birthdayNotifyBlock(ok, 'selected')).toBeNull()
  })
  it('blocks by reason, in backend order', () => {
    expect(birthdayNotifyBlock(null, 'friends')).toBe('required')
    expect(birthdayNotifyBlock({ birthday: null }, 'private')).toBe('required')
    expect(birthdayNotifyBlock({ ...ok, birthday_visibility: 'private' }, 'private')).toBe('private')
    expect(birthdayNotifyBlock(ok, 'private')).toBe('listPrivate')
  })
})
