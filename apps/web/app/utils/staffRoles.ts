// 營運角色（可複選）：admin 隱含全部；support 只有共用唯讀頁
export const STAFF_ROLES = ['admin', 'finance', 'moderator', 'support'] as const
export type StaffRole = typeof STAFF_ROLES[number]
export type StaffUser = { staff_roles?: string[] | null } | null | undefined

export const hasStaffRole = (user: StaffUser, role: StaffRole) =>
  !!user?.staff_roles?.some(r => r === role || r === 'admin')
