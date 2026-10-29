<script setup lang="ts">
const route = useRoute()
const handle = String(route.params.handle)
const { api, base } = useApi()
const { user, fetchMe } = useAuth()
const p = ref<any>(null), state = ref<'loading' | 'ok' | 'gone' | 'error'>('loading')
const busy = ref(false), err = ref('')
const { t, locale } = useI18n()
const visLabel = (v: string) => (['public', 'friends', 'selected', 'link'].includes(v) ? t(`me.profile.vis.${v}`) : v)
const statusLabel = (s: string) => CONTRIBUTION_STATUS[s] ?? (['reserved', 'purchased', 'delivered'].includes(s) ? t(`me.claims.status.${s}`) : s)
const dateLocale = computed(() => (locale.value === 'zh-Hant' ? 'zh-TW' : locale.value))
useHead(() => ({ title: p.value ? t('me.profile.title', { name: p.value.user.display_name }) : t('me.profile.page'), meta: [{ name: 'robots', content: 'noindex' }] }))

// 匿名也要能看：不走 useApi 的 401 轉址，直接帶 cookie 呼叫
async function load() {
  try { p.value = await $fetch(`${base}/users/${encodeURIComponent(handle)}`, { credentials: 'include', headers: nativeHeaders() }); state.value = 'ok' }
  catch (e: any) { state.value = e.status === 404 || e.statusCode === 404 ? 'gone' : 'error' }
}
onMounted(async () => { await fetchMe(); await load() })

async function run(f: () => Promise<any>) {
  busy.value = true; err.value = ''
  try { await f(); await load() } catch (e) { err.value = apiErrMsg(e as any) } finally { busy.value = false }
}
const add = () => run(() => api('/friends/requests', { method: 'POST', body: { to: handle } }))
// 個人頁沒給申請 id，從 /friends 找對方送來的那筆
const { refresh: refreshReqCount } = useFriendRequests()
const accept = () => run(async () => {
  const f = await api('/friends')
  const r = f.incoming.find((x: any) => x.user.id === p.value.user.id)
  if (r) await api(`/friends/requests/${r.id}/accept`, { method: 'POST' })
  refreshReqCount()
})
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="p ? p.user.display_name : $t('me.profile.page')" back="/friends" />
    <p v-if="state === 'loading'" class="c-mute" role="status">{{ $t('me.loading') }}</p>
    <div v-else-if="state === 'gone'" class="c-center"><p>{{ $t('me.profile.gone') }}</p></div>
    <div v-else-if="state === 'error'" class="c-center"><p class="c-err" role="alert">{{ $t('me.profile.loadFailed') }}</p><button class="c-btn" @click="load">{{ $t('me.retry') }}</button></div>
    <template v-else>
      <section class="pf-hero">
        <span class="pf-kick" aria-hidden="true">PROFILE / @{{ p.user.handle }}</span>
        <div class="pf-av" aria-hidden="true">{{ (p.user.display_name || '?').slice(0, 1) }}</div>
        <h2 class="c-h2">{{ p.user.display_name }}</h2>
        <p class="c-mute">@{{ p.user.handle }}<template v-if="p.user.birthday">{{ $t('me.profile.birthday', { date: formatBirthday(p.user.birthday) }) }}</template></p>
        <p v-if="p.user.bio" style="white-space:pre-line">{{ p.user.bio }}</p>
        <p v-if="err" class="c-err" role="alert">{{ err }}</p>
        <NuxtLink v-if="p.relation === 'self'" to="/settings" class="c-btn">{{ $t('me.profile.editSettings') }}</NuxtLink>
        <span v-else-if="p.relation === 'friend'" class="c-badge active">{{ $t('me.profile.friend') }}</span>
        <span v-else-if="p.relation === 'outgoing'" class="c-badge">{{ $t('me.profile.requestSent') }}</span>
        <button v-else-if="p.relation === 'incoming'" class="c-btn primary" :disabled="busy" @click="accept">{{ $t('me.profile.acceptRequest') }}</button>
        <button v-else-if="p.relation === 'none'" class="c-btn primary" :disabled="busy" @click="add">{{ $t('me.profile.addFriend') }}</button>
        <NuxtLink v-else :to="{ path: '/login', query: { redirect: route.fullPath } }" class="c-btn primary">{{ $t('me.profile.loginToAdd') }}</NuxtLink>
      </section>

      <section>
        <h2 class="c-h2 pf-sec"><span aria-hidden="true">01</span>{{ $t('me.profile.lists') }}</h2>
        <p v-if="!p.wishlists.length" class="c-mute">{{ $t('me.profile.noLists') }}</p>
        <article v-for="w in p.wishlists" :key="w.slug" class="c-card pf-card">
          <div class="c-row">
            <img class="c-thumb" :src="coverSrc(w)" alt="">
            <div class="c-grow">
              <NuxtLink :to="`/s/${w.slug}`"><strong>{{ w.title }}</strong></NuxtLink>
              <div class="c-row wrap">
                <span class="c-badge">{{ visLabel(w.visibility) }}</span>
                <span v-if="w.status === 'closed'" class="c-badge">{{ $t('me.profile.closed') }}</span>
                <span class="c-mute">{{ $t('me.profile.personal') }}</span>
              </div>
              <div class="c-mute">{{ $t('me.profile.completion', { pct: w.completion_pct ?? 0, n: w.item_count }, w.item_count) }}</div>
            </div>
          </div>
        </article>
      </section>

      <section>
        <h2 class="c-h2 pf-sec"><span aria-hidden="true">02</span>{{ $t('me.profile.donations') }}</h2>
        <p v-if="!p.donations.length" class="c-mute">{{ $t('me.profile.noDonations') }}</p>
        <article v-for="(d, i) in p.donations" :key="i" class="c-card pf-card">
          <strong>{{ d.item_title }}{{ d.kind === 'pledge' ? $t('me.profile.pledgePts', { pts: d.points }) : $t('me.profile.qty', { qty: d.qty }) }}</strong>
          <span class="c-badge">{{ statusLabel(d.status) }}</span>
          <div class="c-mute">
            {{ $t('me.profile.to', { name: d.wishlist.owner.display_name }) }}
            <NuxtLink v-if="d.wishlist.slug" :to="`/s/${d.wishlist.slug}`">{{ d.wishlist.title }}</NuxtLink><template v-else>{{ d.wishlist.title }}</template>
            ・{{ new Date(d.created_at).toLocaleDateString(dateLocale) }}
          </div>
        </article>
      </section>
    </template>
  </main>
</template>

<style scoped>
.pf-kick{display:block;margin:0 0 8px;font:700 11px var(--mono);letter-spacing:.16em;color:var(--mute)}
.pf-kick::before{content:"";display:inline-block;width:8px;height:8px;background:var(--red);margin-right:8px}
.pf-hero{border:1px solid var(--ink-line);border-top:3px solid var(--red);padding:24px 20px;margin:16px 0 8px;display:grid;gap:10px;justify-items:start;background:var(--card)}
.pf-hero .c-h2{font-size:clamp(28px,6vw,40px);font-weight:900;letter-spacing:-.02em;margin:0;line-height:1.1}
.pf-hero p{margin:0}
.pf-hero .c-h2::before{display:none}
.pf-av{width:64px;height:64px;display:grid;place-items:center;background:var(--fg);color:var(--bg);font:900 28px var(--font)}
.pf-sec{font:700 12px var(--mono);letter-spacing:.14em;text-transform:uppercase;margin:32px 0 12px;padding-top:12px;border-top:1px solid var(--ink-line)}
.pf-sec span{color:var(--red-text);margin-right:8px}
.pf-card{border-top:2px solid var(--ink-line)!important;transition:transform var(--t) var(--ease),box-shadow var(--t) var(--ease)}
@media (hover:hover){.pf-card:hover{transform:translate(-2px,-2px);box-shadow:6px 6px 0 var(--bg-alt),6px 6px 0 1px var(--line)}}
</style>
