import { pickToast } from '../utils/inboxStream'
// 站內通知：未讀數、最新 5 筆、toast。由 plugins/inbox.client.ts 的串流驅動 refresh()；
// 失敗保留前值。noRedirect：未登入或 401 都不能把頁面導去登入。version 每次更新遞增，通知頁據此重抓。
export function useInbox() {
  const unread = useState<number>('inbox-unread', () => 0)
  const latest = useState<any[]>('inbox-latest', () => [])
  const version = useState<number>('inbox-version', () => 0)
  const toast = useState<any>('inbox-toast', () => null)
  const seen = useState<{ top: string | null, unread: number } | null>('inbox-seen', () => null)
  const { api } = useApi()
  let busy = false, again = false
  // 同時間只抓一次，期間又有訊號就結束後再抓一次：一連串訊號不會變成一連串請求，回應也不會亂序
  async function refresh() {
    if (busy) { again = true; return }
    busy = true
    try {
      const r = await api('/me/notifications', { query: { limit: 5 }, noRedirect: true })
      toast.value = pickToast(seen.value, r.items, r.unread) ?? toast.value
      seen.value = { top: r.items[0]?.id ?? null, unread: r.unread }
      latest.value = r.items; unread.value = r.unread; version.value++
    } catch {}
    busy = false
    if (again) { again = false; return refresh() }
  }
  async function readOne(n: any) {
    if (n.read_at) return
    try { await api(`/me/notifications/${n.id}/read`, { method: 'POST' }); n.read_at = new Date().toISOString(); unread.value = Math.max(0, unread.value - 1) } catch {}
  }
  async function readAll() {
    try { await api('/me/notifications/read-all', { method: 'POST' }); const now = new Date().toISOString(); latest.value.forEach(n => { n.read_at ||= now }); unread.value = 0 } catch {}
  }
  const reset = () => { unread.value = 0; latest.value = []; toast.value = null; seen.value = null }
  return { unread, latest, version, toast, refresh, readOne, readAll, reset }
}
