// 站內通知未讀數（鈴鐺徽章用）；不輪詢，失敗保留前值。noRedirect：未登入或 401 都不能把頁面導去登入
export function useInbox() {
  const unread = useState<number>('inbox-unread', () => 0)
  const { api } = useApi()
  async function refresh() {
    try { unread.value = (await api('/me/notifications/unread-count', { noRedirect: true })).unread } catch {}
  }
  return { unread, refresh }
}
