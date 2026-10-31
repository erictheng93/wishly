import { isNative } from './native'

const VID_KEY = 'ws_vid'
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

// localStorage 不可用（私密模式、被封鎖）時回 undefined，後端以 IP 計不重複訪客
function visitorId(): string | undefined {
  try {
    const v = localStorage.getItem(VID_KEY)
    if (v && UUID.test(v)) return v
    const n = crypto.randomUUID()
    localStorage.setItem(VID_KEY, n)
    return n
  } catch { return undefined }
}

function referrerHost(): string | undefined {
  try {
    const h = new URL(document.referrer).hostname
    return h && h !== location.hostname ? h : undefined
  } catch { return undefined }
}

/** 分享頁瀏覽紀錄：只送非原生殼、非範例清單；所有錯誤吞掉，絕不影響分享頁。 */
export function trackShareView(apiBase: string, slug: string): void {
  if (isNative() || slug.startsWith('demo')) return
  try {
    void fetch(`${apiBase}/api/v1/public/wishlists/${slug}/views`, {
      method: 'POST',
      credentials: 'omit',
      keepalive: true,
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ visitor_id: visitorId(), referrer_host: referrerHost() }),
    }).catch(() => {})
  } catch { /* 同步丟錯（如 fetch 不存在）也忽略 */ }
}

/** /me 失敗時呼叫：只有確認 401（未登入）才記錄；5xx、網路錯誤無法確定身分，不記。 */
export function trackIfAnonymous(e: any, apiBase: string, slug: string): void {
  if ((e?.statusCode ?? e?.response?.status ?? e?.status) === 401) trackShareView(apiBase, slug)
}
