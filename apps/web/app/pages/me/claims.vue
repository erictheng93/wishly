<script setup lang="ts">
import '~/assets/guest.css'
const { t } = useI18n()
useSeoMeta({ title: () => t('me.claims.title'), robots: 'noindex' })
const { api } = useGuest()
const state = ref<'loading' | 'ok' | 'empty' | 'error'>('loading')
const guest = ref<any>(null), rows = ref<any[]>([]), recoverFailed = ref(false), msg = ref('')
const editing = ref(false), nick = ref('')
const label = (s: string) => t(`me.claims.status.${s}`)
// 清單結束後仍可標記已購買 / 已送達或取消，但不能改數量、不能延長期限（CTO 決議放寬 Q2）
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
// Q12：購買期限延長 7 天，一次為限
const extend = (r: any) => guard(async () => { Object.assign(r.claim, (await api(`/claims/${r.claim.id}`, { method: 'PATCH', body: { extend: true } })).claim) })
const cancel = (r: any) => {
  const text = t(r.claim.status === 'purchased' ? 'me.claims.confirmCancelPurchased' : 'me.claims.confirmCancel')
  if (confirm(text)) guard(async () => { await api(`/claims/${r.claim.id}`, { method: 'DELETE' }); r.claim.status = 'cancelled' })
}
// Q13：建立者改過品項內容，捐助者按「知道了」後隱藏
const ackChange = (r: any) => guard(async () => { await api(`/claims/${r.claim.id}`, { method: 'PATCH', body: { ack_item_change: true } }); r.item_changed_at = null })
const setVis =(r: any, ev: Event) => guard(async () => { Object.assign(r.claim, (await api(`/claims/${r.claim.id}`, { method: 'PATCH', body: { visibility: (ev.target as HTMLSelectElement).value } })).claim) })
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
    <div class="cl-head"><small aria-hidden="true">WISHLY / CLAIMS</small><h1>{{ $t('me.claims.title') }}</h1></div>
    <div v-if="recoverFailed" class="g-banner err" role="alert">{{ $t('me.claims.recoverFailed') }}</div>
    <div v-if="msg" class="g-banner err" role="alert">{{ msg }}</div>
    <p v-if="state === 'loading'" class="g-mute cl-loading" role="status">{{ $t('me.loading') }}</p>
    <div v-else-if="state === 'error'" class="g-center cl-state bad"><p>{{ $t('me.claims.loadFailed') }}</p><button class="g-btn" @click="load">{{ $t('me.retry') }}</button></div>
    <div v-else-if="state === 'empty'" class="g-center cl-state">
      <p>{{ $t('me.claims.empty1') }}</p>
      <p class="g-mute">{{ $t('me.claims.empty2') }}</p>
    </div>
    <template v-else>
      <div class="g-card" v-if="guest">
        <template v-if="!editing"><b>{{ guest.display_name }}</b>{{ guest.is_user ? $t('me.claims.signedIn') : $t('me.claims.thisDevice') }}<button v-if="!guest.is_user" class="g-link" @click="editing = true">{{ $t('me.claims.editNick') }}</button></template>
        <form v-else class="g-inline" @submit.prevent="saveNick"><input v-model="nick" maxlength="30" required :aria-label="$t('me.claims.nick')"><button class="g-btn sm">{{ $t('me.claims.save') }}</button></form>
      </div>
      <section v-for="(g, gi) in groups" :key="g.w.slug" class="cl-group">
        <h2 class="g-h2 cl-h2" :data-n="String(gi + 1).padStart(2, '0')"><NuxtLink :to="`/s/${g.w.slug}`">{{ g.w.title }}</NuxtLink></h2>
        <div v-for="r in g.list" :key="r.claim.id" class="g-card g-item">
          <img v-if="r.item.image_url" class="g-thumb" :src="r.item.image_url" alt=""><div v-else class="g-thumb">{{ $t('me.claims.noImg') }}</div>
          <div class="g-body">
            <div class="g-title">{{ r.item.title }} × {{ r.claim.qty }}<span class="g-badge">{{ label(r.claim.status) }}</span></div>
            <div v-if="r.item_changed_at" class="g-banner js-item-changed" role="status">{{ $t('me.claims.itemChanged', { time: fmtTime(r.item_changed_at) }) }}<button class="g-link" @click="ackChange(r)">{{ $t('me.claims.ackItemChange') }}</button></div>
            <label v-if="r.claim.visibility" class="g-mute g-vis">{{ $t('me.whoSees') }}
              <select :value="r.claim.visibility" @change="setVis(r, $event)"><option value="public">{{ $t('me.vis.public') }}</option><option value="friends">{{ $t('me.vis.friends') }}</option><option value="private">{{ $t('me.vis.private') }}</option></select>
            </label>
            <div v-if="r.claim.status === 'reserved' && r.claim.expires_at" class="g-mute js-deadline">{{ $t('me.claims.deadline', { date: fmtTime(r.claim.expires_at) }) }}</div>
            <div v-if="closed(r.wishlist) && r.claim.status !== 'delivered'" class="g-mute">{{ $t('me.claims.closedHint') }}</div>
            <div class="g-actions">
              <button v-if="r.claim.status === 'reserved'" class="g-btn sm" @click="setStatus(r, 'purchased')">{{ $t('me.claims.markPurchased') }}</button>
              <button v-if="r.claim.status === 'purchased'" class="g-btn sm" @click="setStatus(r, 'delivered')">{{ $t('me.claims.markDelivered') }}</button>
              <button v-if="r.claim.status === 'reserved' && r.claim.expires_at && !r.claim.extended_at && !closed(r.wishlist)" class="g-btn ghost sm" @click="extend(r)">{{ $t('me.claims.extend') }}</button>
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
<style scoped>
.cl-head{margin:28px 0 24px;padding-bottom:16px;border-bottom:1px solid var(--ink-line);animation:rise .5s var(--ease) both}
.cl-head small{display:block;margin-bottom:6px;font:600 11px/1 var(--mono);letter-spacing:.16em;color:var(--red-text)}
.cl-head h1{margin:0}
.cl-state{display:grid;justify-items:center;gap:10px;margin:8px 0 0;padding:44px 20px;text-align:center;border:1px dashed var(--line);background:var(--hatch)}
.cl-state p{margin:0;max-width:34em;line-height:1.7;text-wrap:balance}
.cl-state.bad{border:1px solid var(--line);border-left:3px solid var(--bad);background:var(--bad-bg)}
.cl-loading{display:flex;align-items:center;min-height:72px;margin:0;padding:0 16px;border:1px solid var(--line);border-left:3px solid var(--ink-line);background:var(--hatch);font:600 12px/1.4 var(--mono);letter-spacing:.14em}
.cl-group{margin-top:36px}
.cl-h2{display:flex;align-items:baseline;gap:12px;margin:0 0 6px;padding-bottom:10px;border-bottom:1px solid var(--ink-line);font-size:20px}
.cl-h2::before{content:attr(data-n);font:600 11px/1 var(--mono);letter-spacing:.14em;color:var(--red-text)}
.cl-h2 a{display:inline-flex;align-items:center;min-height:44px}
.cl-group .g-item{margin:0;border-width:0 0 1px;border-color:var(--line);padding:16px 0}
.cl-group .g-thumb{width:80px;height:80px}
.cl-group .g-item:hover{box-shadow:none;transform:none}
@media (max-width:767px){.cl-head{margin-top:20px}}
</style>
