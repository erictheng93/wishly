<script setup lang="ts">
import '~/assets/guest.css'
const { t } = useI18n()
useSeoMeta({ title: () => t('me.claims.title'), robots: 'noindex' })
const { api } = useGuest()
const state = ref<'loading' | 'ok' | 'empty' | 'error'>('loading')
const guest = ref<any>(null), rows = ref<any[]>([]), recoverFailed = ref(false), msg = ref('')
const editing = ref(false), nick = ref('')
const label = (s: string) => t(`me.claims.status.${s}`)
// Q2：清單結束後捐助者只能取消（後端回 409 WISHLIST_CLOSED）
const closed = (w: any) => ['closed', 'archived'].includes(w.status)
const active = computed(() => rows.value.filter(r => ['reserved', 'purchased', 'delivered'].includes(r.claim.status)))
const past = computed(() => rows.value.filter(r => !active.value.includes(r)))
const groups = computed(() => {
  const m = new Map<string, any>()
  for (const r of active.value) (m.get(r.wishlist.slug) ?? m.set(r.wishlist.slug, { w: r.wishlist, list: [] }).get(r.wishlist.slug)).list.push(r)
  return [...m.values()]
})

async function load() {
  state.value = 'loading'
  try {
    const r = await api('/guest/me?limit=100')
    guest.value = r.guest; rows.value = r.claims; nick.value = r.guest.display_name
    state.value = rows.value.length ? 'ok' : 'empty'
  } catch (e: any) {
    if (e.status === 401) { setGuestToken(null); state.value = 'empty' } else state.value = 'error'
  }
}
onMounted(async () => {
  const m = location.hash.match(/^#r=(.+)$/)
  if (m) {
    history.replaceState(null, '', location.pathname) // 一次性權杖，立即從網址移除
    try { setGuestToken((await api('/guest/recover', { method: 'POST', body: { token: m[1] } })).guest_token) } catch { recoverFailed.value = true }
  }
  load()
})

const guard = async (f: () => Promise<any>) => { msg.value = ''; try { await f() } catch (e: any) { msg.value = e.detail } }
const setStatus = (r: any, status: string) => guard(async () => { Object.assign(r.claim, (await api(`/claims/${r.claim.id}`, { method: 'PATCH', body: { status } })).claim) })
const cancel = (r: any) => {
  const text = t(r.claim.status === 'purchased' ? 'me.claims.confirmCancelPurchased' : 'me.claims.confirmCancel')
  if (confirm(text)) guard(async () => { await api(`/claims/${r.claim.id}`, { method: 'DELETE' }); r.claim.status = 'cancelled' })
}
const setVis = (r: any, ev: Event) => guard(async () => { Object.assign(r.claim, (await api(`/claims/${r.claim.id}`, { method: 'PATCH', body: { visibility: (ev.target as HTMLSelectElement).value } })).claim) })
const saveNick = () => guard(async () => { guest.value = (await api('/guest/me', { method: 'PATCH', body: { display_name: nick.value } })).guest; editing.value = false })
const wipe = () => {
  if (confirm(t('me.claims.confirmWipe'))) guard(async () => {
    await api('/guest/me', { method: 'DELETE' }); setGuestToken(null); guest.value = null; rows.value = []; state.value = 'empty'
  })
}
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <h1>{{ $t('me.claims.title') }}</h1>
    <div v-if="recoverFailed" class="g-banner err" role="alert">{{ $t('me.claims.recoverFailed') }}</div>
    <div v-if="msg" class="g-banner err" role="alert">{{ msg }}</div>
    <p v-if="state === 'loading'" class="g-mute">{{ $t('me.loading') }}</p>
    <div v-else-if="state === 'error'" class="g-center"><p>{{ $t('me.claims.loadFailed') }}</p><button class="g-btn" @click="load">{{ $t('me.retry') }}</button></div>
    <div v-else-if="state === 'empty'" class="g-center">
      <p>{{ $t('me.claims.empty1') }}</p>
      <p class="g-mute">{{ $t('me.claims.empty2') }}</p>
    </div>
    <template v-else>
      <div class="g-card" v-if="guest">
        <template v-if="!editing"><b>{{ guest.display_name }}</b>{{ guest.is_user ? $t('me.claims.signedIn') : $t('me.claims.thisDevice') }}<button v-if="!guest.is_user" class="g-link" @click="editing = true">{{ $t('me.claims.editNick') }}</button></template>
        <form v-else class="g-inline" @submit.prevent="saveNick"><input v-model="nick" maxlength="30" required :aria-label="$t('me.claims.nick')"><button class="g-btn sm">{{ $t('me.claims.save') }}</button></form>
      </div>
      <section v-for="g in groups" :key="g.w.slug">
        <h2 class="g-h2"><NuxtLink :to="`/s/${g.w.slug}`">{{ g.w.title }}</NuxtLink></h2>
        <div v-for="r in g.list" :key="r.claim.id" class="g-card g-item">
          <img v-if="r.item.image_url" class="g-thumb" :src="r.item.image_url" alt=""><div v-else class="g-thumb">{{ $t('me.claims.noImg') }}</div>
          <div class="g-body">
            <div class="g-title">{{ r.item.title }} × {{ r.claim.qty }}<span class="g-badge">{{ label(r.claim.status) }}</span></div>
            <label v-if="r.claim.visibility" class="g-mute g-vis">{{ $t('me.whoSees') }}
              <select :value="r.claim.visibility" @change="setVis(r, $event)"><option value="public">{{ $t('me.vis.public') }}</option><option value="friends">{{ $t('me.vis.friends') }}</option><option value="private">{{ $t('me.vis.private') }}</option></select>
            </label>
            <div class="g-actions">
              <template v-if="!closed(r.wishlist)">
                <button v-if="r.claim.status === 'reserved'" class="g-btn sm" @click="setStatus(r, 'purchased')">{{ $t('me.claims.markPurchased') }}</button>
                <button v-if="r.claim.status === 'purchased'" class="g-btn sm" @click="setStatus(r, 'delivered')">{{ $t('me.claims.markDelivered') }}</button>
              </template>
              <span v-else-if="r.claim.status !== 'delivered'" class="g-mute">{{ $t('me.claims.closedHint') }}</span>
              <button v-if="r.claim.status !== 'delivered'" class="g-btn ghost sm" @click="cancel(r)">{{ $t('me.claims.cancel') }}</button>
            </div>
          </div>
        </div>
      </section>
      <details v-if="past.length"><summary class="g-mute">{{ $t('me.claims.past', { n: past.length }) }}</summary>
        <div v-for="r in past" :key="r.claim.id" class="g-mute">{{ r.item.title }} × {{ r.claim.qty }}・{{ label(r.claim.status) }}</div>
      </details>
      <div v-if="!guest?.is_user" class="g-foot"><button class="g-link" @click="wipe">{{ $t('me.claims.wipe') }}</button></div>
    </template>
  </main>
</template>
