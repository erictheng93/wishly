// 站內通知即時串流（GET /me/notifications/stream）：以 fetch 讀 SSE（EventSource 不能帶 Bearer，網頁與原生共用同一條路）。
// 訊號只是提示（event: inbox），內容一律由呼叫端 onEvent 去打 REST。純函式、不依賴 Vue / Nuxt，方便單元測試。
export interface InboxStreamOpts {
  url: string
  headers: () => Record<string, string>
  onEvent: () => void
  fetchImpl?: typeof fetch
  delays?: number[] // 斷線退避（毫秒）
  failLimit?: number // 連續失敗幾次後改輪詢
  pollMs?: number
  idleMs?: number // 超過這麼久沒收到任何位元組（伺服器每 15 秒有心跳）就視為斷線
  jitter?: number // 重連延遲隨機放大的比例，避免部署重啟後所有客戶端同時湧入
}

export function startInboxStream(o: InboxStreamOpts) {
  const { delays = [2000, 5000, 15000, 30000], failLimit = 3, pollMs = 15000, idleMs = 45000, jitter = 0.5, fetchImpl = (...a) => fetch(...a) } = o
  let stopped = false, ctrl: AbortController | undefined, poll: ReturnType<typeof setInterval> | undefined
  let timer: ReturnType<typeof setTimeout> | undefined, wake: (() => void) | undefined
  const sleep = (ms: number) => new Promise<void>((r) => { wake = r; timer = setTimeout(r, ms) })
  const stopPoll = () => { clearInterval(poll); poll = undefined }

  async function run() {
    let fails = 0
    while (!stopped) {
      let ok = false
      try {
        ctrl = new AbortController()
        const r = await fetchImpl(o.url, { headers: { Accept: 'text/event-stream', ...o.headers() }, credentials: 'include', cache: 'no-store', signal: ctrl.signal })
        if (r.status === 401 || r.status === 403) return stop() // 沒登入或被拒：不重試
        if (r.status === 429 || r.status === 503) fails = failLimit // 名額已滿：不用再試三次才知道，先輪詢兜底
        if (!r.ok || !r.body) throw new Error(String(r.status))
        ok = true; fails = 0; stopPoll(); o.onEvent() // 每次連上補一次，涵蓋斷線期間的通知
        const reader = r.body.getReader(), dec = new TextDecoder()
        let buf = ''
        for (;;) {
          // 休眠或換網路時連線可能無聲卡死，read 永遠不回；沒有看門狗就再也收不到通知
          const idle = setTimeout(() => ctrl?.abort(), idleMs)
          const { done, value } = await reader.read().finally(() => clearTimeout(idle))
          if (done) break
          buf += dec.decode(value, { stream: true }).replace(/\r\n?/g, '\n')
          let i: number
          while ((i = buf.indexOf('\n\n')) >= 0) {
            const block = buf.slice(0, i); buf = buf.slice(i + 2)
            if (block.split('\n').some(l => l.replace(/^event: ?/, '') === 'inbox' && l.startsWith('event:'))) o.onEvent()
          }
        }
      } catch { /* 斷線或被 stop 中止，走下面的重連 */ }
      if (stopped) return
      if (!ok && ++fails >= failLimit) poll ??= setInterval(o.onEvent, pollMs) // 連不上時用輪詢兜底
      await sleep(delays[Math.min(ok ? 0 : fails - 1, delays.length - 1)]! * (1 + Math.random() * jitter))
    }
  }
  function stop() { stopped = true; ctrl?.abort(); stopPoll(); clearTimeout(timer); wake?.() }
  void run()
  return { stop }
}

/** 最新一筆（id 變了、未讀、未讀數增加）才跳 toast；首次載入（prev 為 null）不跳 */
export function pickToast(prev: { top: string | null, unread: number } | null, items: any[], unread: number) {
  const n = items[0]
  return prev && n && n.id !== prev.top && !n.read_at && unread > prev.unread ? n : null
}
