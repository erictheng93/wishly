// 生日只存月日 'MM-DD'（見 0014_profile_fields）；用閏年 2000 算天數，2 月可選 29
export const daysInMonth = (m: string) => m ? new Date(2000, +m, 0).getDate() : 31

// 月日都空 → null（清除）；只填一邊 → undefined（不合法）；日超過該月天數時取該月最後一天
export function toBirthday(m: string, d: string): string | null | undefined {
  if (!m && !d) return null
  if (!m || !d) return undefined
  return `${m}-${String(Math.min(+d, daysInMonth(m))).padStart(2, '0')}`
}

export const formatBirthday = (b: string) => `${+b.slice(0, 2)} 月 ${+b.slice(3)} 日`
