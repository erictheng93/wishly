<script setup lang="ts">
import '~/assets/guest.css'
// 公開分享頁：必須 SSR，LINE / FB 爬蟲才讀得到 OG（docs 03 §5.6）
const route = useRoute()
const { t, locale } = useI18n()
const slug = route.params.slug as string
// 範例清單（db/seed/demo.sql）：頂端提供互相切換，一般清單不顯示
const demos = slug.startsWith('demo') ? [['demobirth0', t('guest.slug.demoBirthday')], ['demogift00', t('guest.slug.demoGift')], ['demowedd00', t('guest.slug.demoWedding')]] : []
const { public: { apiBase } } = useRuntimeConfig()
const { api } = useGuest()
const listUrl = `${apiBase}/api/v1/public/wishlists/${slug}`

// 不用 useFetch 的 error 拋出：404 / 410 在頁內顯示，並設正確 HTTP 狀態碼（410 需 noindex、不洩漏標題）
// cache: 'no-cache'：API 回的 Cache-Control 帶 stale-while-revalidate（給 CDN 用），瀏覽器會先吐舊資料；捐助後的 refresh() 必須是新的（仍會用 ETag 回 304）
const { data: fetched, error, refresh } = await useFetch<any>(`${apiBase}/api/v1/public/wishlists/${slug}`, { credentials: 'omit', cache: 'no-cache' })
// 404 找不到 / 410 下架 / 其他（5xx、網路、逾時）= 暫時無法載入，回 503，避免爬蟲與快取把暫時性錯誤記成「不存在」
// 暫時性錯誤（5xx / 網路）時保留上次成功的內容，不要讓 15 秒輪詢的一次失敗把整頁換成錯誤畫面；404 / 410 才清空
const data = ref<any>(fetched.value)
// 受限清單（好友 / 指定 / 密碼）：SSR 沒有帶 cookie 會拿到 403（或擁有者的私人清單 404），client 掛載後帶憑證重抓才知道真正結果。
// st = 目前結果的 HTTP 狀態；gate = 403 的 body（含 code / owner）；checking = client 重抓中（先不顯示 gate，避免閃一下）
const st = ref<number | null>(error.value?.statusCode ?? null)
const gate = ref<any>(null), checking = ref(st.value === 403)
const restricted = ref(false)
const hold = (c: number | null) => c === 403 && !gate.value // SSR 的 403 不是最終結果，等 client 重抓
watch([fetched, error], ([v, e]) => {
  if (v) { data.value = v; st.value = null } else if (e) { st.value = e.statusCode as number; if ([404, 410].includes(st.value!)) data.value = null; if (st.value === 403) loadClient() }
})
async function loadClient() {
  try {
    // 受限清單 / 擁有者：帶憑證打不走 CDN 快取的路徑（公開路徑永遠只回 public 內容）
    const r = await fetchListWithCreds(apiBase, slug)
    data.value = r; st.value = null; gate.value = null; restricted.value = isRestricted(r.visibility)
    startLive()
  } catch (e: any) {
    st.value = e?.statusCode ?? e?.response?.status ?? 0
    if (st.value === 403) { gate.value = e.data ?? {}; restricted.value = true; data.value = null }
    else if ([404, 410].includes(st.value!)) data.value = null
  } finally { checking.value = false }
}
// 受限清單用帶憑證的 $fetch 更新；公開清單維持 useFetch refresh（ETag）
const reload = () => restricted.value ? loadClient() : refresh()
const gone = computed(() => st.value === 410)
const notFound = computed(() => st.value === 404)
const forbidden = computed(() => st.value === 403 && !!gate.value)
const failed = computed(() => !!st.value && !gone.value && !notFound.value && st.value !== 403)
if (error.value && import.meta.server) setResponseStatus(useRequestEvent()!, gone.value ? 410 : notFound.value ? 404 : st.value === 403 ? 200 : 503)

if (data.value) {
  // 規格：docs 03 §5.6 meta 內容規則
  const left = () => (data.value?.items ?? []).filter((i: any) => !i.is_fully_claimed).length
  const origin = useRequestURL().origin
  useSeoMeta({
    title: () => data.value?.title,
    ogTitle: () => t('guest.slug.ogTitle', { name: data.value?.owner?.display_name ?? t('guest.slug.friend'), title: data.value?.title }),
    description: () => data.value?.description ?? undefined,
    ogDescription: () => t('guest.slug.ogDesc', { pct: data.value?.completion?.completion_pct ?? 0, left: left() }),
    // ponytail: 預設圖為靜態檔；動態 /og/{slug}.png（含清單名稱與完成度）之後再做
    ogImage: () => data.value?.cover_image_url ?? `${origin}/og-default.png`,
    ogImageWidth: 1200, ogImageHeight: 630,
    ogType: 'website', ogUrl: () => `${origin}/s/${slug}`, ogLocale: () => ({ 'zh-Hant': 'zh_TW', en: 'en_US', vi: 'vi_VN' } as Record<string, string>)[String(locale.value)], twitterCard: 'summary_large_image',
  })
} else useSeoMeta({ title: () => data.value?.title ?? (st.value === 403 ? t('guest.slug.titleForbidden') : gone.value ? t('guest.slug.titleGone') : failed.value ? t('guest.slug.titleFailed') : t('guest.slug.titleNotFound')), robots: 'noindex' }) // 受限清單 client 重抓成功後標題改成清單名

useHead({ noscript: [{ innerHTML: '<style>.js-claim{display:none!important}.g-nojs{display:block!important}</style>' }] })
const items = ref<any[]>(data.value?.items ?? [])
watch(data, v => { if (v) items.value = v.items })
// 剩餘優先 → 已滿排最後（穩定排序）
const isDone = (i: any) => i.funding_mode === 'crowdfund' ? deriveDisplayStatus(i) !== 'open' : i.is_fully_claimed
const sorted = computed(() => [...items.value].sort((a, b) => Number(isDone(a)) - Number(isDone(b))))
const pct = computed(() => data.value?.completion?.completion_pct ?? 0)
const closed = computed(() => data.value?.status === 'closed')

// --- 線上狀態 / LINE 引導 ---
const online = ref(true), mode = ref<LiveMode>('connecting'), lineHint = ref(false)
// --- 我的捐助（有 token 時 GET /guest/me）---
const myClaims = ref<Record<string, any>>({})
async function loadMine() {
  try { // 不先檢查 token：登入使用者靠 session cookie 捐助，沒有 guest token
    const r = await api('/guest/me?limit=100')
    myClaims.value = Object.fromEntries(r.claims.filter((c: any) => c.wishlist.slug === slug && ['reserved', 'purchased', 'delivered'].includes(c.claim.status)).map((c: any) => [c.claim.item_id, c.claim]))
  } catch { /* 401：token 失效，當作沒有 */ }
}
// 建立者本人 + 驚喜鎖定中 → 軟性遮蔽：看得到數量，不能捐助；捐助者姓名由 API 在鎖定期間就不輸出（GET /me 比對 owner.id）
const isOwner = ref(false)
const me = ref<any>(null) // 已登入使用者：捐助以帳號名稱進行，不再問暱稱
const masked = computed(() => isOwner.value && !!data.value?.surprise_locked)

let live: ReturnType<typeof createLiveRefresh> | undefined
// 認捐需登入：開啟前先等 /me 回來；未登入（含只有舊訪客 token）一律導去登入並帶回本頁。SSR 一律先當未登入渲染（按鈕為登入連結），client 確認登入後才換成「我要送」
let meReady: Promise<void> = Promise.resolve()
const loginTo = computed(() => me.value ? undefined : loginUrl(route.fullPath))
const toLogin = () => navigateTo(loginUrl(route.fullPath))
// 認捐被回 401 = session 已失效：清掉本頁與全域（ClaimSheet 的 fetchMe 會快取）的登入狀態，否則登入頁會以為還登入著而把人彈回來
const auth = useAuth()
function onLoginExpired() { sheetItem.value = null; me.value = null; auth.user.value = null; toLogin() }
async function openClaim(it: any) { await meReady; if (!me.value) return toLogin(); sheetItem.value = it }
onMounted(async () => {
  online.value = navigator.onLine
  addEventListener('online', () => { online.value = true; reload() }); addEventListener('offline', () => (online.value = false))
  lineHint.value = isLineBrowser() && !getPref('ws_hint_dismissed')
  loadMine()
  meReady = $fetch<any>(`${apiBase}/api/v1/me`, { credentials: 'include', headers: nativeHeaders() }).then(m => { me.value = m; isOwner.value = m?.id === data.value?.owner?.id }).catch(() => {})
  if (hold(st.value) || st.value === 404) await loadClient()
  startLive()
  nextTick(focusItem)
})
// 深連結 ?item=<id>：項目渲染後（僅 client，不影響 SSR / CDN 快取）捲到該卡並加亮
const focusId = route.query.item
let focused = false
function focusItem() {
  if (focused || typeof focusId !== 'string') return
  const el = document.getElementById(`item-${focusId}`)
  if (!el) return
  focused = true
  el.scrollIntoView({ block: 'center' }); el.classList.add('g-hl')
}
watch(() => items.value.length, () => nextTick(focusItem))
function startLive() {
  if (live || !data.value) return
  live = createLiveRefresh({
    url: `${apiBase}/api/v1/public/wishlists/${slug}/events`,
    withCredentials: restricted.value,
    refresh: reload, onMode: m => (mode.value = m),
    onEvent: (type, d) => {
      if (type === 'wishlist.updated' || !d) return reload()
      if (d.deleted) items.value = items.value.filter(i => i.id !== d.item_id)
      else {
        items.value = items.value.map(i => i.id === d.item_id ? mergeItemEvent(i, d) : i)
        if (items.value.some(i => i.id === d.item_id && i.funding_mode === 'crowdfund')) refresh() // contributors 不在事件裡，補抓一次
      }
    },
  })
  live.start()
}
onBeforeUnmount(() => { live?.stop() })


// --- 捐助 sheet / 成功 overlay ---
const sheetItem = ref<any>(null), done = ref<any>(null), reporting = ref(false), toast = ref('')
const sheetMine = computed(() => sheetItem.value ? myClaims.value[sheetItem.value.id] ?? null : null)
function onDone(r: any) {
  myClaims.value = { ...myClaims.value, [r.claim.item_id]: r.claim }
  items.value = items.value.map(i => i.id === r.item.id ? { ...i, ...r.item, is_fully_claimed: r.item.qty_remaining <= 0, progress_percent: Math.min(100, Math.round(r.item.qty_claimed / r.item.qty_needed * 100)) } : i)
  done.value = { ...r, title: sheetItem.value.title }
  sheetItem.value = null; reload()
}
// --- 點數贊助（需登入：未登入導到登入頁，登入後回到本頁）---
const fundId = ref<string | null>(null), funded = ref<any>(null)
const fundItem = computed(() => items.value.find(i => i.id === fundId.value) ?? null) // 取即時資料（SSE / refresh 會換掉物件）
async function onFund(it: any) {
  await meReady
  if (!me.value) return toLogin()
  fundId.value = it.id
}
function onFunded(r: any) {
  items.value = items.value.map(i => i.id === r.item.id ? { ...i, ...r.item } : i)
  funded.value = { ...r, title: fundItem.value?.title }
  fundId.value = null; refresh()
}
async function onSwitchEdit() { await loadMine(); const it = sheetItem.value; sheetItem.value = null; await nextTick(); sheetItem.value = it }
function onStale(rem: number) { items.value = items.value.map(i => i.id === sheetItem.value?.id ? { ...i, qty_remaining: rem, is_fully_claimed: rem <= 0 } : i) }
function dismissHint() { setPref('ws_hint_dismissed', '1'); lineHint.value = false }
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <div v-if="gone" class="g-center"><h1>{{ $t('guest.slug.titleGone') }}</h1><p class="g-mute">{{ $t('guest.slug.goneBody') }}</p><NuxtLink to="/">{{ $t('guest.home') }}</NuxtLink></div>
    <div v-else-if="hold(st) || checking" class="g-center g-mute" role="status">{{ $t('guest.loading') }}</div>
    <div v-else-if="forbidden" class="g-center">
        <h1>{{ gate.code === 'LOGIN_REQUIRED' ? $t('guest.slug.loginRequired') : $t('guest.slug.noPermission') }}</h1>
        <p class="g-mute">{{ apiErrMsg({ code: gate.code, status: 403 }) }}<template v-if="gate.owner">{{ $t('guest.slug.owner', { name: gate.owner.display_name }) }}</template></p>
        <NuxtLink v-if="gate.code === 'LOGIN_REQUIRED'" class="g-btn" :to="{ path: '/login', query: { redirect: `/s/${slug}` } }">{{ $t('guest.slug.loginToView') }}</NuxtLink>
        <NuxtLink v-else-if="gate.code === 'FRIENDS_ONLY' && gate.owner?.handle" class="g-btn" :to="`/u/${gate.owner.handle}`">{{ $t('guest.slug.addFriend', { name: gate.owner.display_name }) }}</NuxtLink>
      <p><NuxtLink to="/">{{ $t('guest.home') }}</NuxtLink></p>
    </div>
    <div v-else-if="failed && !data" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="reload()">{{ $t('guest.reload') }}</button></div>
    <div v-else-if="!data" class="g-center"><h1>{{ $t('guest.slug.titleNotFound') }}</h1><NuxtLink to="/">{{ $t('guest.home') }}</NuxtLink></div>
    <template v-else>
      <div class="g-banner g-nojs">{{ $t('guest.slug.noJs') }}</div>
      <div v-if="!online" class="g-banner">{{ $t('guest.slug.offline') }}</div>
      <div v-if="lineHint" class="g-banner">{{ $t('guest.slug.lineHint') }}<button class="g-link" @click="dismissHint">{{ $t('guest.slug.gotIt') }}</button></div>
      <div v-if="closed" class="g-banner">{{ $t('guest.slug.closed') }}</div>
      <nav v-if="demos.length" class="g-demos" :aria-label="$t('guest.slug.demosAria')">{{ $t('guest.slug.demosLabel') }}<NuxtLink v-for="[k, label] in demos" :key="k" :to="`/s/${k}`" :aria-current="k === slug ? 'page' : undefined">{{ label }}</NuxtLink></nav>
      <img v-if="data.cover_image_url" :src="data.cover_image_url" alt="" class="g-cover">
      <h1>{{ data.title }}</h1>
      <p class="g-mute">
        <template v-if="data.event_date">{{ $t('guest.slug.date', { date: data.event_date }) }}</template><template v-if="data.closes_at && data.status === 'active'">{{ $t('guest.slug.endsAt', { time: new Date(data.closes_at).toLocaleString(intlLocale(), { dateStyle: 'medium', timeStyle: 'short' }) }) }}</template>{{ $t('guest.slug.createdBy', { name: data.owner.display_name }) }}
      </p>
      <p v-if="data.description">{{ data.description }}</p>
      <div v-if="masked" class="g-banner">{{ $t('guest.slug.maskedBanner') }}</div>
      <p v-else-if="data.surprise_locked" class="g-mute">{{ $t('guest.slug.surpriseHint') }}</p>
      <div class="g-mute">{{ $t('guest.slug.completion', { pct, done: data.completion.fulfilled_count, total: data.completion.item_count }) }}</div>
      <div class="g-bar" role="progressbar" :aria-valuenow="pct" aria-valuemin="0" aria-valuemax="100"><i :style="{ width: pct + '%' }" /></div>
      <p v-if="!items.length" class="g-center g-mute">{{ $t('guest.slug.empty') }}</p>
      <p v-if="loginTo && items.length && !closed" class="g-mute">{{ $t('guest.slug.loginHint') }}<NuxtLink :to="loginTo">{{ $t('guest.slug.login') }}</NuxtLink></p>
      <ul class="c-list g-grid">
        <GuestItemCard v-for="it in sorted" :key="it.id" :item="it" :mine="myClaims[it.id] ?? null" :masked="masked" 
          :closed="closed" :online="online" :login-to="loginTo" @claim="openClaim(it)" @fund="onFund(it)" />
      </ul>
      <div class="g-foot">
        <div><span class="g-dot" :class="{ on: mode !== 'connecting' }" />{{ liveLabel(mode) }}</div>
        <button class="g-link" @click="reporting = true">{{ $t('guest.report.title') }}</button>
        ・<NuxtLink to="/terms">{{ $t('guest.slug.terms') }}</NuxtLink>・<NuxtLink to="/privacy">{{ $t('guest.slug.privacy') }}</NuxtLink>
      </div>
    </template>

    <GuestClaimSheet v-if="sheetItem" :key="sheetItem.id + (sheetMine?.id ?? '')" :item="sheetItem" :mine="sheetMine" :online="online" :logged-in-name="me?.display_name"
      @close="sheetItem = null" @done="onDone" @stale="onStale" @switch-edit="onSwitchEdit" @login="onLoginExpired" />
    <GuestFundSheet v-if="fundItem" :key="fundItem.id" :item="fundItem" :slug="slug" :online="online" :logged-in-name="me?.display_name"
      @close="fundId = null" @done="onFunded" @stale="refresh()" />
    <GuestDialog v-if="funded" labelledby="funded-title" @close="funded = null"><div class="g-sheet center">
      <h1 id="funded-title" class="ok">{{ $t('guest.slug.fundedTitle') }}</h1>
      <p>{{ $t('guest.slug.fundedLine', { title: funded.title, pts: fmtPts(funded.contribution.points) }) }}</p>
      <div v-if="funded.funded" class="g-banner ok">{{ $t('guest.slug.fundedGoal') }}</div>
      <p v-if="funded.wallet" class="g-mute">{{ $t('guest.slug.walletLeft', { n: fmtPts(funded.wallet.balance) }) }}</p>
      <NuxtLink class="g-btn block" to="/me/wallet">{{ $t('guest.slug.viewWallet') }}</NuxtLink>
      <button class="g-link" @click="funded = null">{{ $t('guest.slug.backToList') }}</button>
    </div></GuestDialog>
    <GuestReportSheet v-if="reporting" :slug="slug" @close="reporting = false" @sent="reporting = false; toast = $t('guest.report.sent')" />

    <GuestDialog v-if="done" labelledby="done-title" @close="done = null"><div class="g-sheet center">
      <h1 id="done-title" class="ok">{{ $t('guest.slug.doneTitle') }}</h1>
      <p>{{ $t('guest.slug.doneLine', { title: done.title, qty: done.claim.qty }) }}</p>
      <div class="g-banner ok">{{ $t('guest.slug.doneHint') }}</div>
      <div v-if="isLineBrowser()" class="g-banner">{{ $t('guest.slug.lineBrowser') }}</div>
      <NuxtLink class="g-btn block" to="/me/claims">{{ $t('guest.slug.viewMyClaims') }}</NuxtLink>
      <button class="g-link" @click="done = null">{{ $t('guest.slug.backToList') }}</button>
    </div></GuestDialog>
    <div v-if="toast" class="g-banner g-toast" role="status" @click="toast = ''">{{ toast }}</div>
  </main>
</template>
