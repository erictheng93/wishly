// 支持品項歸因：記住「從哪個支持品項進來」，7 天內在這張清單捐助才計入。via 來自 URL（不可信），必須是 UUID 才寫入。
const TTL = 7 * 24 * 3600 * 1000
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const key = (slug: string) => `ws_via:${slug}`

export function rememberVia(slug: string, raw: unknown, now = Date.now()): void {
  if (typeof raw !== 'string' || !UUID.test(raw)) return
  try { localStorage.setItem(key(slug), JSON.stringify({ id: raw, at: now })) } catch { /* 無痕或停用儲存：不歸因 */ }
}

export function currentVia(slug: string, now = Date.now()): string | undefined {
  try {
    const v = JSON.parse(localStorage.getItem(key(slug)) ?? 'null')
    if (typeof v?.id === 'string' && UUID.test(v.id) && typeof v.at === 'number' && now - v.at <= TTL) return v.id
  } catch { /* 損毀當作沒有 */ }
}
