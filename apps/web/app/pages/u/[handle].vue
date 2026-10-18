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
const accept = () => run(async () => {
  const f = await api('/friends')
  const r = f.incoming.find((x: any) => x.user.id === p.value.user.id)
  if (r) await api(`/friends/requests/${r.id}/accept`, { method: 'POST' })
})
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="p ? p.user.display_name : $t('me.profile.page')" back="/friends" />
    <p v-if="state === 'loading'" class="c-mute" role="status">{{ $t('me.loading') }}</p>
    <div v-else-if="state === 'gone'" class="c-center"><p>{{ $t('me.profile.gone') }}</p></div>
    <div v-else-if="state === 'error'" class="c-center"><p class="c-err" role="alert">{{ $t('me.profile.loadFailed') }}</p><button class="c-btn" @click="load">{{ $t('me.retry') }}</button></div>
    <template v-else>
      <section class="c-card">
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
        <h2 class="c-h2">{{ $t('me.profile.lists') }}</h2>
        <p v-if="!p.wishlists.length" class="c-mute">{{ $t('me.profile.noLists') }}</p>
        <article v-for="w in p.wishlists" :key="w.slug" class="c-card">
          <div class="c-row">
            <img v-if="w.cover_image_url" class="c-thumb" :src="w.cover_image_url" alt="">
            <div class="c-grow">
              <NuxtLink :to="`/s/${w.slug}`"><strong>{{ w.title }}</strong></NuxtLink>
              <div class="c-row wrap">
                <span class="c-badge">{{ visLabel(w.visibility) }}</span>
                <span v-if="w.status === 'closed'" class="c-badge">{{ $t('me.profile.closed') }}</span>
                <span class="c-mute">{{ w.type === 'registry' ? $t('me.profile.registry') : $t('me.profile.personal') }}</span>
              </div>
              <div class="c-mute">{{ $t('me.profile.completion', { pct: w.completion_pct ?? 0, n: w.item_count }) }}</div>
            </div>
          </div>
        </article>
      </section>

      <section>
        <h2 class="c-h2">{{ $t('me.profile.donations') }}</h2>
        <p v-if="!p.donations.length" class="c-mute">{{ $t('me.profile.noDonations') }}</p>
        <article v-for="(d, i) in p.donations" :key="i" class="c-card">
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
