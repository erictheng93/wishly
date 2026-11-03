import { describe, expect, it } from 'vitest'
import { activityLabelKey } from '../app/utils/securityActivity'

describe('activityLabelKey', () => {
  it('已知動作換成底線 key', () => expect(activityLabelKey('auth.login')).toBe('settings.activity.a.auth_login'))
  it('未知動作用通用 key', () => expect(activityLabelKey('x.y')).toBe('settings.activity.a.other'))
})
