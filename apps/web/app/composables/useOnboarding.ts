// 新手導覽（D66）：個人資料 → 建立清單 → 加品項 → 發佈（受捐人）→ 捐助（捐助人）。
// 步驟完成與否由後端從資料推導（GET /me/onboarding）；這裡只決定「目前步驟要帶去哪、在哪一頁顯示提示」
export type TourStep = 'profile' | 'wishlist' | 'item' | 'publish' | 'give'
export interface TourState { done: boolean; current: TourStep | null; wishlist_id: string | null; steps: { key: TourStep; done: boolean }[] }

export const TOUR_STEPS: TourStep[] = ['profile', 'wishlist', 'item', 'publish', 'give']

/** 目前步驟要去的頁面；加品項 / 發佈帶到後端挑的清單，沒有可用的清單就去建一份 */
export function tourTarget(step: TourStep | null, wishlistId?: string | null): string {
  switch (step) {
    case 'profile': return '/welcome'
    case 'wishlist': return '/lists/new'
    case 'item': case 'publish': return wishlistId ? `/lists/${wishlistId}/edit` : '/lists/new'
    case 'give': return '/explore'
    default: return '/welcome'
  }
}

/** 是否已在該步驟要操作的頁面（顯示「怎麼做」而非「前往」）；捐助在探索頁與任何分享頁都算 */
export function onTourPage(step: TourStep | null, path: string, wishlistId?: string | null): boolean {
  if (step === 'give') return path === '/explore' || path.startsWith('/s/')
  if (step === 'item' || step === 'publish') return wishlistId ? path === `/lists/${wishlistId}/edit` : /^\/lists\/[^/]+\/edit$/.test(path)
  return path === tourTarget(step, wishlistId)
}

export function useOnboarding() {
  const state = useState<TourState | null>('onboarding', () => null)
  const { user } = useAuth()
  const { api } = useApi()
  // 只有後端明確回 false 才算進行中：舊版 API 沒有這個欄位時不打擾
  const pending = computed(() => user.value?.onboarding_done === false)
  const active = computed(() => pending.value && !!state.value && !state.value.done)
  async function refresh() {
    if (!pending.value) { state.value = null; return null }
    try { state.value = await api('/me/onboarding', { noRedirect: true }) } catch {} // 失敗保留前值，導覽不擋主要功能
    return state.value
  }
  /** done=true：完成或跳過；false：重新開始（設定頁） */
  async function finish(done = true) {
    state.value = await api('/me/onboarding', { method: 'PATCH', body: { done }, noRedirect: true })
    if (user.value) user.value = { ...user.value, onboarding_done: done }
    return state.value
  }
  return { state, active, pending, refresh, finish }
}
