import { describe, expect, it } from 'vitest'
import { hasStaffRole } from '../app/utils/staffRoles'

describe('hasStaffRole', () => {
  it('admin 隱含全部角色', () => {
    expect(hasStaffRole({ staff_roles: ['admin'] }, 'finance')).toBe(true)
  })
  it('只認自己的角色', () => {
    expect(hasStaffRole({ staff_roles: ['finance'] }, 'finance')).toBe(true)
    expect(hasStaffRole({ staff_roles: ['finance'] }, 'moderator')).toBe(false)
  })
  it('空值或非營運人員為 false', () => {
    expect(hasStaffRole(null, 'support')).toBe(false)
    expect(hasStaffRole({}, 'support')).toBe(false)
    expect(hasStaffRole({ staff_roles: [] }, 'support')).toBe(false)
  })
})
