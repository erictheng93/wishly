import { sseBackoff } from './useLiveRefresh'
// SSE 即時更新；連續失敗 2 次即開輪詢（onEvent('poll')），連上後停止輪詢
export function useWishlistEvents(slug: Ref<string | null | undefined>, onEvent: (type: string) => void, pollMs = 15000) {
  const { base } = useApi()
  const status = ref<'idle' | 'live' | 'polling'>('idle')
  let es: EventSource | null = null, timer: any = null, retry: any = null, errs = 0
  const hidden = () => typeof document !== 'undefined' && document.visibilityState === 'hidden'
  const stopPoll = () => { if (timer) { clearInterval(timer); timer = null } }
  const startPoll = () => { if (!timer) { status.value = 'polling'; timer = setInterval(() => { if (!hidden()) onEvent('poll') }, pollMs) } } // 分頁隱藏時不輪詢
  const onVisible = () => { if (timer && !hidden()) onEvent('poll') } // 回到前景立即補一次
  function close() { clearTimeout(retry); es?.close(); es = null }
  function open() {
    close()
    if (!slug.value) return
    if (typeof EventSource === 'undefined') return startPoll()
    const cur = es = new EventSource(`${base}/public/wishlists/${slug.value}/events`, { withCredentials: true })
    es.onopen = () => { errs = 0; stopPoll(); status.value = 'live' }
    for (const t of ['item.updated', 'wishlist.updated']) es.addEventListener(t, () => onEvent(t))
    es.onerror = () => {
      errs++
      if (errs >= 2) startPoll()
      if (cur.readyState === 2) startPoll() // 伺服器拒絕（404/410）：瀏覽器不再重連，改純輪詢
      else { cur.close(); retry = setTimeout(open, sseBackoff(errs)) } // 自己重連（抖動退避），避免 API 重啟後全體同時湧入
    }
  }
  onMounted(() => { document.addEventListener('visibilitychange', onVisible); watch(slug, open, { immediate: true }) })
  onBeforeUnmount(() => { document.removeEventListener('visibilitychange', onVisible); close(); stopPoll() })
  return { status }
}
