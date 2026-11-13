<script setup lang="ts">
import '~/assets/guest.css' // 推薦區塊用 CatalogGrid（g- 樣式）
import type { GiftPick } from '~/utils/catalog'
const route = useRoute()
const handle = String(route.params.handle)
const { api, base } = useApi()
const { user, fetchMe } = useAuth()
const p = ref<any>(null), state = ref<'loading' | 'ok' | 'gone' | 'error'>('loading')
const busy = ref(false), err = ref('')
const { t, locale } = useI18n()
const visLabel = (v: string) => (['public', 'friends', 'selected'].includes(v) ? t(`me.profile.vis.${v}`) : v)
const statusLabel = (s: string) => CONTRIBUTION_STATUS[s] ?? (['reserved', 'purchased', 'delivered'].includes(s) ? t(`me.claims.status.${s}`) : s)
const dateLocale = computed(() => (locale.value === 'zh-Hant' ? 'zh-TW' : locale.value))
const today = computed(() => localDay())
const nextList = computed(() => (p.value ? upcoming<any>(p.value.wishlists, today.value) : null))
const lists = computed(() => (p.value ? sortLists<any>(p.value.wishlists, today.value) : []))
const nextDays = computed(() => (nextList.value ? daysUntil(nextList.value.event_date, today.value) : 0))

// 連結預覽：只在伺服器端以匿名身分抓公開資料；用 id 的個人頁對匿名是 404，忽略錯誤即可
const config = useRuntimeConfig()
const origin = useRequestURL().origin
const { data: seo } = await useFetch<any>(`${config.public.apiBase}/api/v1/users/${encodeURIComponent(handle)}`, { credentials: 'omit', server: true, lazy: false, key: `profile-seo-${handle}`, headers: { accept: 'application/json' } })
const pname = computed(() => p.value?.user.display_name ?? seo.value?.user?.display_name)
useSeoMeta({
  title: () => (pname.value ? t('me.profile.title', { name: pname.value }) : t('me.profile.page')),
  ogTitle: () => (pname.value ? t('me.profile.title', { name: pname.value }) : t('me.profile.page')),
  ogDescription: () => seo.value?.user?.bio || (pname.value ? t('me.profile.ogDesc', { name: pname.value }) : undefined),
  ogImage: () => seo.value?.user?.avatar_url || `${origin}/og-default.png`,
  twitterCard: 'summary', robots: 'noindex',
})

const copied = ref(false)
async function share(path = 'u') {
  const url = `${config.public.siteUrl}/${path}/${p.value.user.handle}`
  try {
    if (navigator.share) await navigator.share({ title: p.value.user.display_name, url })
    else { await copyText(url); copied.value = true; setTimeout(() => (copied.value = false), 2000) }
  } catch (e: any) { if (e?.name !== 'AbortError') err.value = apiErrMsg(e) }
}

// 匿名也要能看：不走 useApi 的 401 轉址，直接帶 cookie 呼叫
async function load() {
  try { p.value = await $fetch(`${base}/users/${encodeURIComponent(handle)}`, { credentials: 'include', headers: nativeHeaders() }); state.value = 'ok' }
  catch (e: any) {
    const missing = e.status === 404 || e.statusCode === 404
    // 用 id 的個人頁只給登入者（後端對匿名回 404），先導去登入
    if (missing && !user.value && /^[0-9a-f-]{36}$/i.test(handle)) return navigateTo({ path: '/login', query: { redirect: route.fullPath } })
    state.value = missing ? 'gone' : 'error'
  }
}
// 送禮推薦：本人、封鎖、API 失敗或沒有商品都整區不顯示
const picks = ref<{ basis: string; products: GiftPick[] } | null>(null)
async function loadPicks() {
  picks.value = null
  if (!p.value || ['self', 'blocked'].includes(p.value.relation)) return
  try { const r = await fetchGiftPicks(config.public.apiBase, p.value.user.handle ?? p.value.user.id, { limit: 6 }); picks.value = r.products.length ? r : null } catch { picks.value = null }
}
onMounted(async () => {
  await fetchMe(); await load(); loadPicks()
  // 用 id 進來但對方已設 handle：換成好分享的網址
  if (p.value?.user.handle && p.value.user.handle !== handle.toLowerCase()) useRouter().replace(`/u/${p.value.user.handle}`)
})

async function run(f: () => Promise<any>) {
  busy.value = true; err.value = ''
  try { await f(); await load() } catch (e) { err.value = apiErrMsg(e as any) } finally { busy.value = false }
}
const add = () => run(() => api('/friends/requests', { method: 'POST', body: { to: handle } }))
const { refresh: refreshReqCount } = useFriendRequests()
const accept = () => run(async () => { await api(`/friends/requests/${p.value.request_id}/accept`, { method: 'POST' }); refreshReqCount(true) })
const dropRequest = () => run(async () => { await api(`/friends/requests/${p.value.request_id}`, { method: 'DELETE' }); refreshReqCount(true) })
const unfriending = ref(false)
const unfriend = () => run(async () => { await api(`/friends/${p.value.user.id}`, { method: 'DELETE' }); unfriending.value = false })
const blocking = ref(false)
const block = () => run(async () => { await api(`/users/${p.value.user.id}/block`, { method: 'POST' }); blocking.value = false })
const unblock = () => run(() => api(`/users/${p.value.user.id}/block`, { method: 'DELETE' }))
// 檢舉使用者：登入者專用，不走 Turnstile，原因選項沿用清單檢舉的文案
const REASONS = ['scam', 'inappropriate', 'copyright', 'personal_info', 'other']
const reporting = ref(false), rReason = ref('scam'), rDetail = ref(''), rBusy = ref(false), rErr = ref(''), rSent = ref(false)
function openReport() { rReason.value = 'scam'; rDetail.value = ''; rErr.value = ''; reporting.value = true }
async function sendReport() {
  if (rBusy.value) return
  rBusy.value = true; rErr.value = ''
  try {
    await api(`/users/${p.value.user.id}/reports`, { method: 'POST', body: { reason: rReason.value, detail: rDetail.value.trim() || undefined } })
    reporting.value = false; rSent.value = true
  } catch (e) { rErr.value = apiErrMsg(e as any) } finally { rBusy.value = false }
}
</script>
<template>
  <main class="c-page"><ClientOnly>
    <CreatorHeader :title="p ? p.user.display_name : $t('me.profile.page')" :back="user ? '/friends' : '/'" :no-title="!!p" />
    <p v-if="state === 'loading'" class="c-mute" role="status">{{ $t('me.loading') }}</p>
    <section v-else-if="state === 'gone'" class="c-gone"><p class="c-gone-t">{{ $t('me.profile.gone') }}</p></section>
    <section v-else-if="state === 'error'" class="c-gone err"><p class="c-err c-gone-t" role="alert">{{ $t('me.profile.loadFailed') }}</p><button class="c-btn" @click="load">{{ $t('me.retry') }}</button></section>
    <template v-else>
      <img v-if="p.user.cover_image_url || p.user.cover_preset" :src="p.user.cover_image_url || `/covers/${p.user.cover_preset}.svg`" alt="" class="pf-cover">
      <section class="pf-hero">
        <div class="pf-av" aria-hidden="true"><img v-if="p.user.avatar_url" :src="p.user.avatar_url" alt="" class="pf-img"><template v-else>{{ (p.user.display_name || '?').slice(0, 1) }}</template></div>
        <h1 class="c-h2">{{ p.user.display_name }}</h1>
        <p v-if="p.user.handle || p.user.birthday || p.user.age != null" class="c-mute pf-handle"><template v-if="p.user.handle">@{{ p.user.handle }}</template><template v-if="p.user.birthday">{{ $t('me.profile.birthday', { date: formatBirthday(p.user.birthday) }) }}</template><template v-if="p.user.age != null"><template v-if="p.user.handle || p.user.birthday"> · </template>{{ $t('me.profile.age', { n: p.user.age }) }}</template></p>
        <p v-if="p.user.dates?.length" class="c-mute pf-handle pf-date">{{ p.user.dates[0].title }} · {{ formatBirthday(p.user.dates[0].month_day) }} · {{ p.user.dates[0].days_until ? $t('me.profile.inDays', { n: p.user.dates[0].days_until }) : $t('me.profile.dateToday') }}</p>
        <p v-if="p.user.bio" class="pf-bio">{{ p.user.bio }}</p>
        <p v-if="p.mutual?.count" class="c-mute c-row wrap">
          <NuxtLink v-for="m in p.mutual.users" :key="m.id" :to="profilePath(m)" class="c-row"><span class="pf-av pf-av-s" aria-hidden="true"><img v-if="m.avatar_url" :src="m.avatar_url" alt="" class="pf-img"><template v-else>{{ (m.display_name || '?').slice(0, 1) }}</template></span>{{ m.display_name }}</NuxtLink>
          <span>{{ $t('me.profile.mutual', { n: p.mutual.count }, p.mutual.count) }}</span>
        </p>
        <div class="c-row wrap pf-act">
        <NuxtLink v-if="p.relation === 'self'" to="/settings" class="c-btn">{{ $t('me.profile.editSettings') }}</NuxtLink>
        <template v-else-if="p.relation === 'blocked'">
          <span class="c-mute">{{ $t('me.profile.blocked') }}</span>
          <button class="c-btn" :disabled="busy" @click="unblock">{{ $t('me.profile.unblock') }}</button>
        </template>
        <template v-else-if="p.relation === 'friend'">
          <span class="c-badge active">{{ $t('me.profile.friend') }}</span>
          <button class="c-btn" :disabled="busy" @click="unfriending = true">{{ $t('me.profile.unfriend') }}</button>
        </template>
        <template v-else-if="p.relation === 'outgoing'">
          <span class="c-badge">{{ $t('me.profile.requestSent') }}</span>
          <button class="c-btn" :disabled="busy" @click="dropRequest">{{ $t('me.profile.cancelRequest') }}</button>
        </template>
        <template v-else-if="p.relation === 'incoming'">
          <button class="c-btn primary" :disabled="busy" @click="accept">{{ $t('me.profile.acceptRequest') }}</button>
          <button class="c-btn" :disabled="busy" @click="dropRequest">{{ $t('me.profile.decline') }}</button>
        </template>
        <button v-else-if="p.relation === 'none'" class="c-btn primary" :disabled="busy" @click="add">{{ $t('me.profile.addFriend') }}</button>
        <NuxtLink v-else :to="{ path: '/login', query: { redirect: route.fullPath } }" class="c-btn primary">{{ $t('me.profile.loginToAdd') }}</NuxtLink>
        <button v-if="p.user.handle && p.relation !== 'blocked'" class="c-btn" @click="share()">{{ $t('me.profile.share') }}</button>
        <button v-if="p.user.handle && (p.relation === 'self' || p.user.links?.length)" class="c-btn" @click="share('l')">{{ $t('me.profile.shareLinks') }}</button>
        <span v-if="copied" class="c-mute" role="status">{{ $t('me.profile.copied') }}</span>
        </div>
        <div v-if="user && !['self', 'anonymous', 'blocked'].includes(p.relation)" class="c-row wrap">
          <button class="pf-quiet" :disabled="busy" @click="blocking = true">{{ $t('me.profile.block') }}</button>
          <button class="pf-quiet" @click="openReport">{{ $t('me.profile.report') }}</button>
        </div>
        <p v-if="rSent" class="c-ok" role="status">{{ $t('me.profile.reportSent') }}</p>
        <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      </section>

      <section v-if="nextList" class="pf-next">
        <span class="pf-kick">{{ $t('me.profile.next') }} · {{ nextDays ? $t('me.profile.nextDays', { n: nextDays }) : $t('me.profile.nextToday') }}</span>
        <NuxtLink :to="`/s/${nextList.slug}`"><strong>{{ nextList.title }}</strong></NuxtLink>
      </section>

      <CreatorProfileSections v-if="p.relation !== 'blocked'" :user="p.user" />

      <section v-if="picks" data-gift-picks>
        <h2 class="c-h2 pf-sec">{{ $t(picks.basis === 'interests' ? 'me.profile.picksTitle' : 'me.profile.picksFallback', { name: p.user.display_name }) }}</h2>
        <GuestCatalogGrid compact :products="picks.products" :matched="picks.basis === 'interests' ? matchedMap(picks.products) : undefined" />
      </section>

      <section v-if="p.relation !== 'blocked'">
        <h2 class="c-h2 pf-sec">{{ $t('me.profile.lists') }}</h2>
        <template v-if="!p.wishlists.length">
          <p class="c-mute">{{ $t('me.profile.noLists') }}</p>
          <NuxtLink v-if="p.relation === 'self'" to="/lists/new" class="c-btn primary">{{ $t('me.profile.createList') }}</NuxtLink>
        </template>
        <article v-for="w in lists" :key="w.slug" class="c-card pf-card">
          <div class="c-row">
            <img class="c-thumb" :src="coverSrc(w)" alt="">
            <div class="c-grow">
              <NuxtLink :to="`/s/${w.slug}`"><strong>{{ w.title }}</strong></NuxtLink>
              <div class="c-row wrap">
                <span class="c-badge">{{ visLabel(w.visibility) }}</span>
                <span v-if="w.status === 'closed'" class="c-badge">{{ $t('me.profile.closed') }}</span>
              </div>
              <div class="c-mute">{{ $t('me.profile.completion', { pct: w.completion_pct ?? 0, n: w.item_count }, w.item_count) }}</div>
            </div>
          </div>
        </article>
      </section>

      <section v-if="p.relation !== 'blocked'">
        <h2 class="c-h2 pf-sec">{{ $t('me.profile.donations') }}</h2>
        <p v-if="!p.donations.length" class="c-mute">{{ $t('me.profile.noDonations') }}</p>
        <component :is="p.relation === 'self' ? 'div' : 'details'" v-if="p.donations.length">
          <summary v-if="p.relation !== 'self'" class="c-mute pf-sum">{{ $t('me.profile.sent', { n: p.donations.length }) }}</summary>
          <article v-for="(d, i) in p.donations" :key="i" class="c-card pf-card">
          <strong>{{ d.item_title }}{{ d.kind === 'pledge' ? $t('me.profile.pledgePts', { pts: d.points }) : $t('me.profile.qty', { qty: d.qty }) }}</strong>
          <span v-if="p.relation === 'self'" class="c-badge">{{ statusLabel(d.status) }}</span>
          <div class="c-mute">
            <NuxtLink v-if="d.wishlist.owner.handle" :to="`/u/${d.wishlist.owner.handle}`">{{ $t('me.profile.to', { name: d.wishlist.owner.display_name }) }}</NuxtLink><template v-else>{{ $t('me.profile.to', { name: d.wishlist.owner.display_name }) }}</template>・
            <NuxtLink v-if="d.wishlist.slug" :to="`/s/${d.wishlist.slug}`">{{ d.wishlist.title }}</NuxtLink><template v-else>{{ d.wishlist.title }}</template>
            ・{{ new Date(d.created_at).toLocaleDateString(dateLocale) }}
          </div>
        </article>
        </component>
      </section>
      <CreatorConfirm :open="unfriending" :title="$t('me.profile.unfriendTitle', { name: p.user.display_name })" :text="$t('me.profile.unfriendText')" :ok="$t('me.profile.unfriend')" danger :busy="busy" :error="err" @close="unfriending = false" @ok="unfriend" />
      <CreatorConfirm :open="blocking" :title="$t('me.profile.blockTitle', { name: p.user.display_name })" :text="$t('me.profile.blockText')" :ok="$t('me.profile.block')" danger :busy="busy" :error="err" @close="blocking = false" @ok="block" />
      <CreatorSheet :open="reporting" :title="$t('me.profile.reportTitle', { name: p.user.display_name })" @close="reporting = false">
        <form @submit.prevent="sendReport">
          <fieldset class="pf-fs"><legend class="c-mute">{{ $t('guest.report.reasonLegend') }}</legend>
            <label v-for="v in REASONS" :key="v" class="c-row"><input v-model="rReason" type="radio" :value="v"> {{ $t('guest.report.reason.' + v) }}</label>
          </fieldset>
          <label class="c-field"><span>{{ $t('guest.report.detailLabel') }}</span><textarea v-model="rDetail" rows="3" maxlength="1000" :placeholder="$t('guest.report.placeholder')" /></label>
          <p v-if="rErr" class="c-err" role="alert">{{ rErr }}</p>
          <div class="c-row">
            <button type="button" class="c-btn c-grow" @click="reporting = false">{{ $t('creator.confirm.cancel') }}</button>
            <button class="c-btn primary c-grow" :disabled="rBusy">{{ rBusy ? $t('creator.confirm.busy') : $t('guest.report.send') }}</button>
          </div>
        </form>
      </CreatorSheet>
    </template>
  </ClientOnly></main>
</template>

<style scoped>
.pf-cover{display:block;width:100%;height:140px;object-fit:cover;background:var(--bg-alt)}
.pf-kick{display:block;margin:0 0 8px;font:700 12px var(--mono);letter-spacing:.04em;color:var(--mute)}
.pf-hero{border-bottom:1px solid var(--ink-line);padding:32px 0 24px;margin:0;display:grid;grid-template-columns:72px minmax(0,1fr);gap:2px 16px;align-items:center}
.pf-hero>*{grid-column:1/-1}
.pf-hero>.pf-av{grid-row:1/3;grid-column:1;width:72px;height:72px;font-size:32px}
.pf-hero>.c-h2{grid-row:1;grid-column:2;align-self:end;font-size:clamp(28px,7vw,40px);font-weight:900;letter-spacing:-.02em;margin:0;line-height:1.1}
html:lang(vi) .pf-hero>.c-h2{line-height:1.2}
.pf-hero>.c-h2::before{display:none}
.pf-hero>p{margin:0}
.pf-hero>.pf-handle{grid-row:2;grid-column:2;align-self:start;font:500 13px var(--mono);letter-spacing:.04em}
.pf-hero>.pf-date{grid-row:auto;grid-column:1/-1;margin-top:8px}
.pf-bio{white-space:pre-line;margin-top:12px!important}
.pf-hero>.c-row{margin-top:14px}
.pf-hero>.pf-act{margin-top:18px}
.pf-av{display:grid;place-items:center;background:var(--fg);color:var(--bg);font:900 28px var(--font);overflow:hidden}
.pf-img{width:100%;height:100%;object-fit:cover}
.pf-av-s{width:24px;height:24px;font-size:12px;margin-right:6px}
.pf-next{border:1px solid var(--line);padding:14px 16px;margin:20px 0 0;background:var(--bg-alt);display:grid;gap:4px}
.pf-next strong{font-size:20px;letter-spacing:-.01em}
.pf-fs{border:0;padding:0;margin:8px 0;display:grid;gap:6px}
.pf-quiet{background:none;border:0;padding:4px 0;font:inherit;font-size:13px;color:var(--mute);text-decoration:underline;cursor:pointer}
.pf-sum{cursor:pointer;margin-bottom:8px}
.pf-sec{font:800 16px/1.3 var(--font);letter-spacing:0;text-transform:none;margin:40px 0 4px}
.pf-card{border:0!important;border-bottom:1px solid var(--line)!important;background:none;padding:14px 0;margin:0;box-shadow:none}
.pf-card .c-thumb{width:64px;height:64px}
.pf-card strong{font-size:17px}
@media (hover:hover){.pf-card:hover{background:var(--bg-alt)}}
.pf-sec~p.c-mute{padding:16px 0;border-bottom:1px solid var(--line);margin:0}
</style>
