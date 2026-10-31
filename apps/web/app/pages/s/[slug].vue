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
  if (v) { data.value = v; st.value = null } else if (e) { st.value = e.statusCode as number; noteUnavailable(st.value); if ([404, 410].includes(st.value!)) data.value = null; if (st.value === 403) loadClient() }
})
// 5xx / 連不到 → 全站繁忙橫幅。每次失敗都要通知（連續同一個狀態碼不會觸發 st 的 watcher，橫幅到期後就不再出現）
if (st.value != null) noteUnavailable(st.value)
async function loadClient() {
  try {
    // 受限清單 / 擁有者：帶憑證打不走 CDN 快取的路徑（公開路徑永遠只回 public 內容）
    const r = await fetchListWithCreds(apiBase, slug)
    data.value = r; st.value = null; gate.value = null; restricted.value = isRestricted(r.visibility)
    startLive()
  } catch (e: any) {
    st.value = e?.statusCode ?? e?.response?.status ?? 0
    noteUnavailable(st.value!)
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
    ogTitle: () => t('guest.slug.ogTitle', { name: data.value?.owner?.display_name ?? data.value?.org?.display_name ?? t('guest.slug.friend'), title: data.value?.title }),
    description: () => data.value?.description ?? undefined,
    ogDescription: () => t('guest.slug.ogDesc', { pct: data.value?.completion?.completion_pct ?? 0, left: left() }),
    // ponytail: 預設圖為靜態檔；動態 /og/{slug}.png（含清單名稱與完成度）之後再做
    // 預設封面用同名 PNG（scripts/cover-og.mjs 產生），LINE/FB 不支援 SVG
    ogImage: () => data.value?.cover_image_url ?? (data.value?.cover_preset ? `${origin}${coverSrc(data.value).replace(/\.svg$/, '.png')}` : `${origin}/og-default.png`),
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
const siteOff = computed(() => !!data.value?.site_status && data.value.site_status !== 'active') // 救災據點非 active 一律當暫停
const closed = computed(() => data.value?.status === 'closed')

// --- 線上狀態 / LINE 引導 ---
const online = ref(true), mode = ref<LiveMode>('connecting'), lineHint = ref(false)
// --- 我的捐助（有 token 時 GET /guest/me）---
const myClaims = ref<Record<string, any>>({})
async function loadMine() {
  try { // 不先檢查 token：登入使用者靠 session cookie 捐助，沒有 guest token
    const r = await api('/guest/me?limit=100')
    myClaims.value = Object.fromEntries(r.claims.filter((c: any) => c.wishlist.slug === slug && ['reserved', 'purchased', 'delivered'].includes(c.claim.status)).map((c: any) => [c.claim.item_id, { ...c.claim, item_changed_at: c.item_changed_at }]))
  } catch { /* 401：token 失效，當作沒有 */ }
}
// Q13：建立者改過品項內容的提醒，按「知道了」後隱藏
async function ackChange(id: string) {
  const c = myClaims.value[id]
  try { await api(`/claims/${c.id}`, { method: 'PATCH', body: { ack_item_change: true } }); myClaims.value = { ...myClaims.value, [id]: { ...c, item_changed_at: null } } } catch { /* 下次載入仍會顯示 */ }
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
  meReady = $fetch<any>(`${apiBase}/api/v1/me`, { credentials: 'include', headers: nativeHeaders() }).then(m => { me.value = m; isOwner.value = m?.id === data.value?.owner?.id }).catch(e => trackIfAnonymous(e, apiBase, slug))
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
  useOnboarding().refresh() // 新手導覽：捐助完成，進到最後一步
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
  useOnboarding().refresh() // 點數認捐也算完成「捐助」
}
async function onSwitchEdit() { await loadMine(); const it = sheetItem.value; sheetItem.value = null; await nextTick(); sheetItem.value = it }
function onStale(rem: number) {
  items.value = items.value.map(i => i.id === sheetItem.value?.id ? { ...i, qty_remaining: rem, is_fully_claimed: rem <= 0 } : i)
  if (rem <= 0 && !sheetMine.value) toast.value = t('guest.claim.soldOut') // sheet 1.2 秒後自動關閉，結果留在頁面上
}
// Q9：非好友看到好友限定清單時，直接用擁有者 ID 送好友邀請（擁有者不一定有 handle / 個人頁）
const { api: authApi } = useApi()
const friendReq = ref<'idle' | 'busy' | 'sent'>('idle'), friendErr = ref('')
async function sendFriendRequest() {
  friendReq.value = 'busy'; friendErr.value = ''
  try { await authApi('/friends/requests', { method: 'POST', body: { to: gate.value.owner.id } }); friendReq.value = 'sent' }
  catch (e) { friendErr.value = errMsg(e); friendReq.value = 'idle' }
}
const { context: pwaCtx, dismiss: pwaDismiss, install: pwaInstall } = usePwaContext()
function dismissHint() { setPref('ws_hint_dismissed', '1'); lineHint.value = false }
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <div v-if="gone" class="g-center"><span class="sx-code" aria-hidden="true">410</span><h1>{{ $t('guest.slug.titleGone') }}</h1><p class="g-mute">{{ $t('guest.slug.goneBody') }}</p><NuxtLink to="/">{{ $t('guest.home') }}</NuxtLink></div>
    <div v-else-if="hold(st) || checking" class="g-center g-mute" role="status">{{ $t('guest.loading') }}</div>
    <div v-else-if="forbidden" class="g-center">
        <span class="sx-code" aria-hidden="true">403</span>
        <h1>{{ gate.code === 'LOGIN_REQUIRED' ? $t('guest.slug.loginRequired') : $t('guest.slug.noPermission') }}</h1>
        <p class="g-mute">{{ apiErrMsg({ code: gate.code, status: 403 }) }}<template v-if="gate.owner">{{ $t('guest.slug.owner', { name: gate.owner.display_name }) }}</template></p>
        <NuxtLink v-if="gate.code === 'LOGIN_REQUIRED'" class="g-btn" :to="{ path: '/login', query: { redirect: `/s/${slug}` } }">{{ $t('guest.slug.loginToView') }}</NuxtLink>
        <template v-else-if="gate.code === 'FRIENDS_ONLY' && gate.owner?.id">
          <button class="g-btn" :disabled="friendReq !== 'idle'" @click="sendFriendRequest">{{ $t('guest.slug.sendFriendRequest', { name: gate.owner.display_name }) }}</button>
          <p v-if="friendReq === 'sent'" role="status">{{ $t('guest.slug.friendRequestSent') }}</p>
          <p v-if="friendErr" class="g-banner err" role="alert">{{ friendErr }}</p>
          <p v-if="gate.owner.handle"><NuxtLink class="g-link" :to="`/u/${gate.owner.handle}`">{{ $t('guest.slug.addFriend', { name: gate.owner.display_name }) }}</NuxtLink></p>
        </template>
      <p><NuxtLink to="/">{{ $t('guest.home') }}</NuxtLink></p>
    </div>
    <div v-else-if="failed && !data" class="g-center"><span class="sx-code" aria-hidden="true">ERR</span><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="reload()">{{ $t('guest.reload') }}</button></div>
    <div v-else-if="!data" class="g-center"><span class="sx-code" aria-hidden="true">404</span><h1>{{ $t('guest.slug.titleNotFound') }}</h1><NuxtLink to="/">{{ $t('guest.home') }}</NuxtLink></div>
    <template v-else>
      <div class="g-banner g-nojs">{{ $t('guest.slug.noJs') }}</div>
      <div v-if="!online" class="g-banner">{{ $t('guest.slug.offline') }}</div>
      <div v-if="lineHint" class="g-banner">{{ $t('guest.slug.lineHint') }}<button class="g-link" @click="dismissHint">{{ $t('guest.slug.gotIt') }}</button></div>
      <div v-if="data.status === 'closed'" class="g-banner">{{ $t('guest.slug.closed') }}</div>
      <nav v-if="demos.length" class="g-demos" :aria-label="$t('guest.slug.demosAria')">{{ $t('guest.slug.demosLabel') }}<NuxtLink v-for="[k, label] in demos" :key="k" :to="`/s/${k}`" :aria-current="k === slug ? 'page' : undefined">{{ label }}</NuxtLink></nav>
      <section class="sx-hero">
        <div class="sx-cover"><img :src="coverSrc(data)" alt="" class="g-cover"><span class="sx-tag" aria-hidden="true">WISHLIST / {{ data.status === 'closed' ? 'CLOSED' : 'OPEN' }}</span></div>
        <div class="sx-main">
          <h1>{{ data.title }}</h1>
          <p class="g-mute sx-meta">
            <template v-if="data.event_date">{{ $t('guest.slug.date', { date: data.event_date }) }}</template><template v-if="data.closes_at && data.status === 'active'">{{ $t('guest.slug.endsAt', { time: new Date(data.closes_at).toLocaleString(intlLocale(), { dateStyle: 'medium', timeStyle: 'short', timeZone: 'Asia/Taipei' }) }) }}</template><template v-if="data.owner">{{ $t('guest.slug.createdBy', { name: data.owner.display_name }) }}</template>
          </p>
          <p v-if="data.org" class="g-mute">
            <NuxtLink :to="`/o/${data.org.slug}`">{{ data.org.display_name }}</NuxtLink><span v-if="data.org.verified" class="g-badge">{{ $t('guest.relief.verified') }}</span>
            <template v-if="data.event">・{{ $t('guest.relief.event') }}：<NuxtLink :to="`/e/${data.event.slug}`">{{ data.event.name }}</NuxtLink></template>
          </p>
          <p v-if="data.site_address" class="g-mute">{{ $t('guest.relief.address') }}：<a :href="mapsUrl(data.site_address)" target="_blank" rel="noopener">{{ data.site_address }}</a></p>
          <p v-if="data.description" class="sx-desc">{{ data.description }}</p>
        </div>
        <aside class="sx-gauge">
          <div class="sx-big" aria-hidden="true"><b>{{ pct }}</b><span>%</span></div>
          <div class="g-bar" role="progressbar" :aria-valuenow="pct" aria-valuemin="0" aria-valuemax="100"><i :style="{ width: pct + '%' }" /></div>
          <div class="g-mute sx-cap">{{ $t('guest.slug.completion', { pct, done: data.completion.fulfilled_count, total: data.completion.item_count }) }}</div>
          <dl class="sx-stats" aria-hidden="true">
            <div><dt>ITEMS</dt><dd>{{ String(data.completion.item_count).padStart(2, '0') }}</dd></div>
            <div><dt>DONE</dt><dd>{{ String(data.completion.fulfilled_count).padStart(2, '0') }}</dd></div>
            <div><dt>OPEN</dt><dd>{{ String(Math.max(0, data.completion.item_count - data.completion.fulfilled_count)).padStart(2, '0') }}</dd></div>
          </dl>
        </aside>
      </section>
      <div v-if="siteOff && !closed" class="g-banner" role="status">{{ data.site_status === 'closed' ? $t('guest.relief.closedSite') : $t('guest.relief.paused') }}</div>
      <div v-if="masked" class="g-banner">{{ $t('guest.slug.maskedBanner') }}</div>
      <p v-else-if="data.surprise_locked" class="g-mute">{{ $t('guest.slug.surpriseHint') }}</p>
      <p v-if="!items.length" class="sx-empty g-mute">{{ $t('guest.slug.empty') }}</p>
      <p v-if="loginTo && items.length && !closed" class="g-mute">{{ $t('guest.slug.loginHint') }}<NuxtLink :to="loginTo">{{ $t('guest.slug.login') }}</NuxtLink></p>
      <ul class="c-list g-grid">
        <GuestItemCard v-for="(it, n) in sorted" :key="it.id" :index="n + 1" :item="it" :mine="myClaims[it.id] ?? null" :masked="masked" 
          :closed="closed" :site-off="siteOff" :online="online" :login-to="loginTo" @claim="openClaim(it)" @fund="onFund(it)" @ack="ackChange(it.id)" />
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
      <GuestInstallHint v-if="pwaCtx" :context="pwaCtx" source="fund" @dismiss="pwaDismiss" @install="pwaInstall" />
      <NuxtLink class="g-btn block" to="/me/wallet">{{ $t('guest.slug.viewWallet') }}</NuxtLink>
      <button class="g-link" @click="funded = null">{{ $t('guest.slug.backToList') }}</button>
    </div></GuestDialog>
    <GuestReportSheet v-if="reporting" :slug="slug" @close="reporting = false" @sent="reporting = false; toast = $t('guest.report.sent')" />

    <GuestDialog v-if="done" labelledby="done-title" @close="done = null"><div class="g-sheet center">
      <h1 id="done-title" class="ok">{{ $t('guest.slug.doneTitle') }}</h1>
      <p>{{ $t('guest.slug.doneLine', { title: done.title, qty: done.claim.qty }) }}</p>
      <p v-if="done.claim.expires_at" class="js-done-deadline">{{ $t('guest.slug.doneDeadline', { date: fmtTime(done.claim.expires_at) }) }}</p>
      <div class="g-banner ok">{{ $t('guest.slug.doneHint') }}</div>
      <div v-if="isLineBrowser()" class="g-banner">{{ $t('guest.slug.lineBrowser') }}</div>
      <GuestInstallHint v-if="pwaCtx" :context="pwaCtx" source="claim" @dismiss="pwaDismiss" @install="pwaInstall" />
      <NuxtLink class="g-btn block" to="/me/claims">{{ $t('guest.slug.viewMyClaims') }}</NuxtLink>
      <button class="g-link" @click="done = null">{{ $t('guest.slug.backToList') }}</button>
    </div></GuestDialog>
    <div v-if="toast" class="g-banner g-toast" role="status" @click="toast = ''">{{ toast }}</div>
  </main>
</template>

<style>
/* 不用 scoped：SSR 的 <h1> 不能帶 data-v 屬性（爬蟲與 e2e 以原始 HTML 驗證標題）；選擇器一律 sx- 前綴 */
.sx-code{display:block;font:900 clamp(96px,26vw,200px)/.85 var(--mono);letter-spacing:-.06em;color:transparent;-webkit-text-stroke:2px var(--ink-line);background:var(--hatch);-webkit-background-clip:text;background-clip:text;margin-bottom:20px}
.sx-empty{margin:28px 0;padding:40px 20px;text-align:center;border:1px dashed var(--ink-line);background:var(--hatch),var(--card);font:500 13px/1.7 var(--mono);letter-spacing:.06em}
.g-center .sx-code{margin-inline:auto;font-size:clamp(80px,22vw,160px)}
.sx-hero{display:grid;grid-template-columns:minmax(0,1fr);gap:20px;margin-top:20px;padding-bottom:28px;border-bottom:1px solid var(--ink-line);animation:rise .5s var(--ease) both}
.sx-cover{position:relative}
.sx-cover .g-cover{margin-top:0;max-height:none;aspect-ratio:16/9}
.sx-tag{position:absolute;left:0;bottom:0;padding:5px 10px;background:var(--bg);color:var(--fg);font:700 11px/1.2 var(--mono);letter-spacing:.14em;border-top:1px solid var(--ink-line);border-right:1px solid var(--ink-line)}
.sx-tag::before{content:"";display:inline-block;width:7px;height:7px;background:var(--red);margin-right:8px}
.sx-main h1{margin:0 0 12px;font-weight:900;font-size:clamp(30px,6vw,46px);text-wrap:balance;word-break:keep-all;overflow-wrap:anywhere}
.sx-meta{font-family:var(--mono);font-size:12px;letter-spacing:.06em}
.sx-desc{margin:14px 0 0;line-height:1.75;max-width:38em}
.sx-gauge{border:1px solid var(--ink-line);padding:16px 18px 0;background:var(--card);align-self:start}
.sx-big{display:flex;align-items:baseline;gap:4px;line-height:.9;color:var(--fg)}
.sx-big b{font:900 clamp(72px,18vw,120px)/.9 var(--font);letter-spacing:-.05em;font-variant-numeric:tabular-nums}
.sx-big span{font:800 28px var(--mono);color:var(--red-text)}
.sx-gauge .g-bar{height:12px;margin:14px 0 10px}
.sx-cap{font-family:var(--mono);font-size:12px;letter-spacing:.04em}
.sx-stats{display:grid;grid-template-columns:repeat(3,1fr);margin:14px -18px 0;border-top:1px solid var(--ink-line)}
.sx-stats div{padding:10px 18px;border-left:1px solid var(--line)}
.sx-stats div:first-child{border-left:0}
.sx-stats dt{font:600 10px var(--mono);letter-spacing:.16em;color:var(--mute)}
.sx-stats dd{margin:2px 0 0;font:800 24px var(--mono);font-variant-numeric:tabular-nums}
@media (min-width:900px){
  .sx-hero{grid-template-columns:minmax(0,1fr) 300px;grid-template-areas:"cover cover" "main gauge";gap:28px}
  .sx-cover{grid-area:cover}.sx-main{grid-area:main}.sx-gauge{grid-area:gauge}
  .sx-cover .g-cover{aspect-ratio:21/8}
}
@media (max-width:899px){.sx-main{order:1}.sx-gauge{order:2}}
</style>
