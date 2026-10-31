// Web Push 的瀏覽器端小工具（後端 push.rs）。不依賴 Nuxt，方便單測。
export const isIos = () => /iphone|ipad|ipod/i.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1)
export const isStandalone = () => matchMedia('(display-mode: standalone)').matches || (navigator as any).standalone === true

export const b64urlToBytes = (s: string) => Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/')), c => c.charCodeAt(0))

/** 此裝置目前的推播訂閱（沒有 service worker 或沒訂閱回 null） */
export async function currentPushSub(): Promise<PushSubscription | null> {
  const reg = await navigator.serviceWorker?.getRegistration()
  return (await reg?.pushManager?.getSubscription()) ?? null
}

/** 登出或關閉推播時：通知後端刪除並取消此裝置的訂閱。任何失敗都吞掉，不可阻擋登出。 */
export async function dropPush(api: (path: string, opts: any) => Promise<any>) {
  try {
    const sub = await currentPushSub()
    if (!sub) return
    await api('/me/push-subscriptions', { method: 'DELETE', body: { endpoint: sub.endpoint }, noRedirect: true }).catch(() => {})
    await sub.unsubscribe()
  } catch {}
}
