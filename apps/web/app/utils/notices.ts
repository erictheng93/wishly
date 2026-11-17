export type SysNotice = { id: string, title: string, body: string, level: 'info' | 'warning' | 'critical', locale: string | null, starts_at: string, ends_at: string | null }
const KEY = 'ws_dismissed_notices'
// critical 不可關閉；其餘依語系與已關閉的 id 過濾
export const visibleNotices = (list: SysNotice[] | undefined, locale: string, dismissed: string[]) =>
  (list ?? []).filter(n => (n.locale === null || n.locale === locale) && (n.level === 'critical' || !dismissed.includes(n.id)))
export function readDismissed(): string[] {
  try { const v = JSON.parse(localStorage.getItem(KEY) || '[]'); return Array.isArray(v) ? v : [] } catch { return [] }
}
export function writeDismissed(ids: string[]) {
  try { localStorage.setItem(KEY, JSON.stringify(ids.slice(-50))) } catch { /* 無痕模式等：僅本次有效 */ }
}
