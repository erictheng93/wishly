// 待處理的好友申請數（導覽徽章用）；失敗保留前值
// 60 秒內（同一位使用者）不重打；剛改動資料的呼叫端傳 force。同時進行中的請求共用同一個 promise
const FRESH_MS = 60_000
let inflight: Promise<void> | null = null
export function useFriendRequests() {
  const count = useState<number>('friend-req-count', () => 0)
  const last = useState<{ uid: string | null, at: number }>('friend-req-last', () => ({ uid: null, at: 0 }))
  const { user } = useAuth()
  const { api } = useApi()
  async function refresh(force = false) {
    const uid = user.value?.id ?? null
    if (!force && last.value.uid === uid && Date.now() - last.value.at < FRESH_MS) return
    if (inflight && !force) return inflight // shortcut: 換帳號瞬間的 in-flight 會被共用，下次進頁即校正
    const p: Promise<void> = (async () => {
      try { count.value = (await api('/friends/requests/count', { noRedirect: true })).incoming; last.value = { uid, at: Date.now() } } catch {}
    })().finally(() => { if (inflight === p) inflight = null })
    return (inflight = p)
  }
  return { count, refresh }
}
