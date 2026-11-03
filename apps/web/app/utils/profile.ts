// 個人頁路徑：沒設 handle 的人用 user id（後端 /users/{handle|id} 兩者皆可）
export const profilePath = (u: { id: string, handle?: string | null }) => `/u/${u.handle || u.id}`

type DatedList = { status: string, event_date?: string | null }
const isUpcoming = (w: DatedList, today: string) => w.status === 'active' && !!w.event_date && w.event_date.slice(0, 10) >= today

// 最近的下一個場合（只看上線中、event_date 在今天或之後）
export const upcoming = <T extends DatedList>(ws: T[], today: string): T | null =>
  ws.filter(w => isUpcoming(w, today)).sort((a, b) => a.event_date!.localeCompare(b.event_date!))[0] ?? null

// 有即將到來場合的清單排前面（依日期升冪），其餘維持原順序
export const sortLists = <T extends DatedList>(ws: T[], today: string): T[] => [
  ...ws.filter(w => isUpcoming(w, today)).sort((a, b) => a.event_date!.localeCompare(b.event_date!)),
  ...ws.filter(w => !isUpcoming(w, today)),
]

// 本地日期 YYYY-MM-DD 與到某日的天數（不用 UTC，避免台灣凌晨差一天）
export const localDay = (d = new Date()) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
export const daysUntil = (day: string, today: string) => Math.round((Date.parse(`${day.slice(0, 10)}T00:00:00Z`) - Date.parse(`${today}T00:00:00Z`)) / 864e5)
