/** 安全活動動作 → i18n key；未知動作回通用 key，避免後端新增動作時畫面出現原始字串 */
const KNOWN = new Set([
  'auth.login', 'auth.logout', 'auth.new_device', 'auth.password_changed', 'auth.password_reset',
  'auth.mfa_enabled', 'auth.mfa_verified', 'auth.mfa_failed', 'auth.mfa_recovery_used', 'auth.mfa_recovery_regenerated', 'auth.recovery_regenerated',
  'auth.mfa_reset', 'auth.session_revoked', 'auth.sessions_revoked', 'auth.identity_linked', 'auth.identity_unlinked',
  'account.export', 'account.absorb', 'account.suspended', 'account.restored',
  'user.mfa_reset', 'user.recovery', 'user.identity_unlink', 'wishlist.shipping_address.update',
])
export const activityLabelKey = (action: string): string =>
  KNOWN.has(action) ? `settings.activity.a.${action.replace(/\./g, '_')}` : 'settings.activity.a.other'
