<script setup lang="ts">
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
async function share() {
  const url = `${config.public.siteUrl}/u/${p.value.user.handle}`
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
onMounted(async () => {
  await fetchMe(); await load()
  // 用 id 進來但對方已設 handle：換成好分享的網址
  if (p.value?.user.handle && p.value.user.handle !== handle.toLowerCase()) useRouter().replace(`/u/${p.value.user.handle}`)
})

async function run(f: () => Promise<any>) {
  busy.value = true; err.value = ''
  try { await f(); await load() } catch (e) { err.value = apiErrMsg(e as any) } finally { busy.value = false }
}
const add = () => run(() => api('/friends/requests', { method: 'POST', body: { to: handle } }))
const { refresh: refreshReqCount } = useFriendRequests()
const accept = () => run(async () => { await api(`/friends/requests/${p.value.request_id}/accept`, { method: 'POST' }); refreshReqCount() })
const dropRequest = () => run(async () => { await api(`/friends/requests/${p.value.request_id}`, { method: 'DELETE' }); refreshReqCount() })
const unfriending = ref(false)
const unfriend = () => run(async () => { await api(`/friends/${p.value.user.id}`, { method: 'DELETE' }); unfriending.value = false })
</script>
<template>
  <main class="c-page"><ClientOnly>
    <CreatorHeader :title="p ? p.user.display_name : $t('me.profile.page')" :back="user ? '/friends' : '/'" />
    <p v-if="state === 'loading'" class="c-mute" role="status">{{ $t('me.loading') }}</p>
    <section v-else-if="state === 'gone'" class="c-gone"><span class="pf-kick" aria-hidden="true">PROFILE / 404</span><p class="c-gone-t">{{ $t('me.profile.gone') }}</p></section>
    <section v-else-if="state === 'error'" class="c-gone err"><span class="pf-kick" aria-hidden="true">PROFILE / ERR</span><p class="c-err c-gone-t" role="alert">{{ $t('me.profile.loadFailed') }}</p><button class="c-btn" @click="load">{{ $t('me.retry') }}</button></section>
    <template v-else>
      <section class="pf-hero">
        <span class="pf-kick" aria-hidden="true">PROFILE<template v-if="p.user.handle"> / @{{ p.user.handle }}</template></span>
        <div class="pf-av" aria-hidden="true"><img v-if="p.user.avatar_url" :src="p.user.avatar_url" alt="" class="pf-img"><template v-else>{{ (p.user.display_name || '?').slice(0, 1) }}</template></div>
        <h2 class="c-h2">{{ p.user.display_name }}</h2>
        <p v-if="p.user.handle || p.user.birthday" class="c-mute"><template v-if="p.user.handle">@{{ p.user.handle }}</template><template v-if="p.user.birthday">{{ $t('me.profile.birthday', { date: formatBirthday(p.user.birthday) }) }}</template></p>
        <p v-if="p.user.bio" style="white-space:pre-line">{{ p.user.bio }}</p>
        <p v-if="p.mutual?.count" class="c-mute c-row wrap">
          <NuxtLink v-for="m in p.mutual.users" :key="m.id" :to="profilePath(m)" class="c-row"><span class="pf-av pf-av-s" aria-hidden="true"><img v-if="m.avatar_url" :src="m.avatar_url" alt="" class="pf-img"><template v-else>{{ (m.display_name || '?').slice(0, 1) }}</template></span>{{ m.display_name }}</NuxtLink>
          <span>{{ $t('me.profile.mutual', { n: p.mutual.count }, p.mutual.count) }}</span>
        </p>
        <div class="c-row wrap">
        <NuxtLink v-if="p.relation === 'self'" to="/settings" class="c-btn">{{ $t('me.profile.editSettings') }}</NuxtLink>
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
        <button v-if="p.user.handle" class="c-btn" @click="share">{{ $t('me.profile.share') }}</button>
        <span v-if="copied" class="c-mute" role="status">{{ $t('me.profile.copied') }}</span>
        </div>
        <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      </section>

      <section v-if="nextList" class="pf-next">
        <span class="pf-kick">{{ $t('me.profile.next') }} · {{ nextDays ? $t('me.profile.nextDays', { n: nextDays }) : $t('me.profile.nextToday') }}</span>
        <NuxtLink :to="`/s/${nextList.slug}`"><strong>{{ nextList.title }}</strong></NuxtLink>
      </section>

      <section>
        <h2 class="c-h2 pf-sec"><span aria-hidden="true">01</span>{{ $t('me.profile.lists') }}</h2>
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

      <section>
        <h2 class="c-h2 pf-sec"><span aria-hidden="true">02</span>{{ $t('me.profile.donations') }}</h2>
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
    </template>
  </ClientOnly></main>
</template>

<style scoped>
.pf-kick{display:block;margin:0 0 8px;font:700 11px var(--mono);letter-spacing:.16em;color:var(--mute)}
.pf-kick::before{content:"";display:inline-block;width:8px;height:8px;background:var(--red);margin-right:8px}
.pf-hero{border:1px solid var(--ink-line);border-top:3px solid var(--red);padding:24px 20px;margin:16px 0 8px;display:grid;gap:10px;justify-items:start;background:var(--card)}
.pf-hero .c-h2{font-size:clamp(28px,6vw,40px);font-weight:900;letter-spacing:-.02em;margin:0;line-height:1.1}
.pf-hero p{margin:0}
.pf-hero .c-h2::before{display:none}
.pf-av{width:64px;height:64px;display:grid;place-items:center;background:var(--fg);color:var(--bg);font:900 28px var(--font)}
.pf-img{width:100%;height:100%;object-fit:cover}
.pf-av{overflow:hidden}
.pf-av-s{width:24px;height:24px;font-size:12px;margin-right:6px}
.pf-next{border:1px solid var(--ink-line);border-left:3px solid var(--red);padding:14px 20px;margin:8px 0;background:var(--card);display:grid;gap:4px}
.pf-next strong{font-size:20px}
.pf-sum{cursor:pointer;margin-bottom:8px}
.pf-sec{font:700 12px var(--mono);letter-spacing:.14em;text-transform:uppercase;margin:32px 0 12px;padding-top:12px;border-top:1px solid var(--ink-line)}
.pf-sec span{color:var(--red-text);margin-right:8px}
.pf-card{border-top:2px solid var(--ink-line)!important;transition:transform var(--t) var(--ease),box-shadow var(--t) var(--ease)}
@media (hover:hover){.pf-card:hover{transform:translate(-2px,-2px);box-shadow:6px 6px 0 var(--bg-alt),6px 6px 0 1px var(--line)}}
</style>
