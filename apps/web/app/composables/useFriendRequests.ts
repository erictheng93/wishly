// 待處理的好友申請數（導覽徽章用）；失敗保留前值
export function useFriendRequests() {
  const count = useState<number>('friend-req-count', () => 0)
  const { api } = useApi()
  async function refresh() {
    try { count.value = (await api('/friends/requests/count', { noRedirect: true })).incoming } catch {}
  }
  return { count, refresh }
}
