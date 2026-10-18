<script setup lang="ts">
import '~/assets/guest.css'
// 「心願眾籌」bottom sheet（契約 POST /items/{id}/contributions）。需要登入；Idempotency-Key 於開啟時產生，
// 確定性失敗後換新 key（body 可能已改），網路失敗才沿用同一把重試。
const props = defineProps<{ item: any, online: boolean, slug: string, loggedInName?: string }>()
const emit = defineEmits<{ close: [], done: [r: any], stale: [] }>()
const { api } = useApi()
const { t } = useI18n()
// 預設帶帳號的 default_claim_visibility（分享頁不會預先載入 /me，這裡自己抓）；載入前不送，由伺服器套預設
const vis = ref<string | undefined>()
useAuth().fetchMe().then(u => { vis.value ??= u?.default_claim_visibility }).catch(() => { /* 取不到就讓伺服器套預設 */ })

const key = ref(crypto.randomUUID())
const remaining = computed(() => props.item.remaining_points ?? 0)
const points = ref<number | ''>(Math.min(remaining.value, 500) || remaining.value || '')
const message = ref(''), anon = ref(false)
const balance = ref<number | null>(null)
const busy = ref(false), msg = ref(''), needWallet = ref(false), needLogin = ref(false)
const dirty = computed(() => !!message.value || anon.value)
const chips = computed(() => quickAmounts(remaining.value))

onMounted(async () => { try { balance.value = (await api('/wallet', { noRedirect: true })).wallet.balance } catch { /* 餘額只是提示 */ } })

const valid = computed(() => Number.isInteger(points.value) && (points.value as number) >= 1 && (points.value as number) <= Math.min(10_000_000, remaining.value || 10_000_000))
const after = computed(() => balance.value != null && typeof points.value === 'number' ? balance.value - points.value : null)

async function submit() {
  msg.value = ''; needWallet.value = needLogin.value = false
  if (!valid.value) { msg.value = t('guest.fund.rangeErr', { max: fmtPts(remaining.value) }); return }
  busy.value = true
  try {
    const r = await api(`/items/${props.item.id}/contributions`, {
      method: 'POST', headers: { 'Idempotency-Key': key.value }, noRedirect: true,
      body: { points: points.value, message: message.value.trim() || undefined, is_anonymous: anon.value, visibility: vis.value },
    })
    emit('done', r)
  } catch (e: any) {
    if (e.code === 'UNAUTHORIZED') { needLogin.value = true; msg.value = t('guest.fund.loginExpired') } else {
      const c = contributionErr(e.code, e.body, errMsg(e))
      msg.value = c.msg; needWallet.value = !!c.wallet
      if (c.points) points.value = c.points
      if (e.body?.balance != null) balance.value = e.body.balance
      if (['ITEM_FUNDED', 'FUNDING_EXPIRED', 'CROWDFUND_TARGET_EXCEEDED'].includes(e.code)) emit('stale')
    }
    if (e.code !== 'NETWORK') key.value = crypto.randomUUID()
  } finally { busy.value = false }
}
</script>

<template>
  <GuestDialog labelledby="fund-title" :dirty="dirty" @close="emit('close')">
    <form class="g-sheet" @submit.prevent="submit">
      <div class="g-item">
        <img v-if="item.image_url" class="g-thumb" :src="item.image_url" alt="">
        <div class="g-body"><div id="fund-title" class="g-title">{{ $t('guest.fund.title', { title: item.title }) }}</div>
          <div class="g-mute">{{ $t('guest.fund.short', { n: fmtPts(remaining) }) }}<template v-if="item.funding_deadline">{{ $t('guest.fund.until', { time: fmtTime(item.funding_deadline) }) }}</template></div></div>
      </div>
      <p class="g-mute">{{ $t('guest.fund.asUser', { name: loggedInName || $t('guest.fund.yourAccount') }) }}<template v-if="balance != null">{{ $t('guest.fund.balanceA') }} <b class="g-num">{{ fmtPts(balance) }}</b> {{ $t('guest.fund.balanceB') }}</template>{{ $t('guest.fund.refundNote') }}</p>
      <label for="fp">{{ $t('guest.fund.pointsLabel') }}</label>
      <input id="fp" v-model.number="points" type="number" inputmode="numeric" min="1" :max="remaining || undefined" step="1" :readonly="busy" required>
      <div class="g-chips">
        <button v-for="n in chips" :key="n" type="button" class="g-btn ghost sm" :class="{ on: points === n }" :disabled="busy" @click="points = n">{{ fmtPts(n) }}</button>
        <button type="button" class="g-btn ghost sm" :class="{ on: points === remaining }" :disabled="busy || !remaining" @click="points = remaining">{{ $t('guest.fund.fullRemaining', { n: fmtPts(remaining) }) }}</button>
      </div>
      <p v-if="after != null && after < 0" class="g-field-err">{{ $t('guest.fund.insufficient', { n: fmtPts(-after) }) }}</p>
      <label for="fm">{{ $t('guest.noteLabel') }}</label>
      <input id="fm" v-model="message" maxlength="200" :readonly="busy">
      <label class="g-radio"><input v-model="anon" type="checkbox" :disabled="busy">{{ $t('guest.fund.anon') }}</label>
      <template v-if="vis">
        <label for="fv">{{ $t('guest.fund.visLabel') }}</label>
        <select id="fv" v-model="vis" :disabled="busy"><option value="public">{{ $t('guest.visibility.public') }}</option><option value="friends">{{ $t('guest.visibility.friends') }}</option><option value="private">{{ $t('guest.visibility.private') }}</option></select>
      </template>
      <p class="g-mute">{{ $t('guest.fund.notice') }}</p>
      <div v-if="msg" class="g-banner err" role="alert">{{ msg }}
        <NuxtLink v-if="needWallet" to="/me/wallet">{{ $t('guest.fund.viewWallet') }}</NuxtLink>
        <NuxtLink v-if="needLogin" :to="{ path: '/login', query: { redirect: `/s/${slug}` } }">{{ $t('guest.fund.relogin') }}</NuxtLink>
      </div>
      <button class="g-btn" :disabled="busy || !online">{{ !online ? $t('guest.fund.offline') : busy ? $t('guest.sending') : $t('guest.fund.confirm', { pts: valid ? $t('guest.pts', { n: fmtPts(points as number) }) : '' }) }}</button>
      <button type="button" class="g-link" @click="emit('close')">{{ $t('guest.cancel') }}</button>
    </form>
  </GuestDialog>
</template>
