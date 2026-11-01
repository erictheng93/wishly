<script setup lang="ts">
import '~/assets/guest.css'
// 捐助 / 修改捐助 bottom sheet（docs 03 §5.7）。Idempotency-Key 於開啟時產生，重試沿用。
const props = defineProps<{ item: any, mine: any | null, online: boolean, loggedInName?: string, via?: string }>()
const emit = defineEmits<{ close: [], done: [r: any], stale: [remaining: number], switchEdit: [], login: [] }>()
const { api, newKey } = useGuest()
const { t } = useI18n()

// 登入者：預設帶帳號的 default_claim_visibility（分享頁不會預先載入 /me，這裡自己抓）；載入前不送，由伺服器套預設
const { fetchMe } = useAuth()
const vis = ref<string | undefined>()
if (props.loggedInName) fetchMe().then(u => { vis.value ??= u?.default_claim_visibility })
const key = newKey()
const editing = !!props.mine
// 舊訪客捐助只能減少或取消，不能加量（後端同樣擋）；登入會員可調到剩餘上限
const max = ref(props.loggedInName ? props.item.qty_remaining + (props.mine?.qty ?? 0) : (props.mine?.qty ?? 0))
const qty = ref(Math.min(props.mine?.qty ?? 1, max.value))
const note = ref(props.mine?.note ?? '')
const anon = ref(false)
const busy = ref(false)
const msg = ref('')
const errs = ref<Record<string, string>>({})
const note0 = note.value
const dirty = computed(() => note.value !== note0 || anon.value)

async function submit() {
  errs.value = {}; msg.value = ''
  busy.value = true
  try {
    let r: any
    if (editing) {
      r = await api(`/claims/${props.mine.id}`, { method: 'PATCH', body: { qty: qty.value, note: note.value || null } })
    } else {
      r = await api(`/items/${props.item.id}/claims`, {
        method: 'POST', headers: { 'Idempotency-Key': key }, keepToken: true,
        body: { qty: qty.value, note: note.value || undefined, visibility: vis.value, is_anonymous: anon.value, via_item_id: props.via },
      })
    }
    emit('done', r)
  } catch (e: any) {
    const c = e.code
    if (e.status === 401 && !editing) { emit('login'); return } // 未登入 / 登入已過期：由分享頁導到登入
    if (c === 'ITEM_FULLY_CLAIMED') {
      const rem = (e.data.remaining ?? 0) + (props.mine?.qty ?? 0)
      emit('stale', e.data.remaining ?? 0)
      max.value = rem; qty.value = Math.min(qty.value, rem) || 1
      msg.value = rem > 0 ? t('guest.claim.soldOutN', { n: rem }) : t('guest.claim.soldOut')
      if (rem <= 0) setTimeout(() => emit('close'), 1200)
    } else if (c === 'CLAIM_ALREADY_EXISTS') {
      msg.value = t('guest.claim.exists')
      emit('switchEdit')
    } else if (c === 'VALIDATION_FAILED') {
      for (const x of e.data.errors ?? []) errs.value[x.pointer.slice(1)] = x.detail
      if (!Object.keys(errs.value).length) msg.value = e.detail // 欄位下已有錯誤就不在橫幅重複
    } else if (c === 'IDEMPOTENCY_CONFLICT') msg.value = t('guest.dupRequest')
    else msg.value = e.detail
  } finally { busy.value = false }
}
</script>

<template>
  <GuestDialog labelledby="claim-title" :dirty="dirty" @close="emit('close')">
    <form class="g-sheet" @submit.prevent="submit">
      <div class="g-item">
        <img v-if="item.image_url" class="g-thumb" :src="item.image_url" alt="">
        <div class="g-body"><div id="claim-title" class="g-title">{{ $t(editing ? 'guest.claim.titleEdit' : 'guest.claim.titleNew', { title: item.title }) }}</div>
          <div class="g-mute">{{ $t('guest.claim.remaining', { n: item.qty_remaining }) }}</div></div>
      </div>
      <div class="g-step">
        <button type="button" :aria-label="$t('guest.claim.dec')" :disabled="busy || qty <= 1" @click="qty--">－</button>
        <b>{{ qty }}</b>
        <button type="button" :aria-label="$t('guest.claim.inc')" :disabled="busy || qty >= max" @click="qty++">＋</button>
      </div>
      <div v-if="msg" class="g-banner err" role="alert">{{ msg }}</div>
      <template v-if="!editing">
        <p class="g-mute">{{ $t('guest.claim.asUser', { name: loggedInName }) }}</p>
        <template v-if="vis">
          <label for="cv">{{ $t('guest.claim.visLabel') }}</label>
          <select id="cv" v-model="vis" :disabled="busy"><option value="public">{{ $t('guest.visibility.public') }}</option><option value="friends">{{ $t('guest.visibility.friends') }}</option><option value="private">{{ $t('guest.visibility.private') }}</option></select>
        </template>
      </template>
      <label for="cm">{{ $t('guest.noteLabel') }}</label>
      <input id="cm" v-model="note" maxlength="200" :readonly="busy">
      <div v-if="errs.note" class="g-field-err">{{ errs.note }}</div>
      <!-- 放在留言欄之後：GuestDialog 會把初始焦點給第一個 input -->
      <label v-if="!editing" class="g-radio"><input v-model="anon" type="checkbox" :disabled="busy">{{ $t('guest.claim.anon') }}</label>
      <p class="g-mute">{{ $t('guest.claim.notice') }}<NuxtLink to="/privacy#collect" target="_blank">{{ $t('guest.claim.privacyLink') }}</NuxtLink><span class="g-nowrap">{{ $t('guest.claim.noCheck') }}</span></p>
      <button class="g-btn" :disabled="busy || !online">{{ !online ? $t('guest.claim.offline') : busy ? $t('guest.sending') : editing ? $t('guest.claim.save') : $t('guest.claim.confirm') }}</button>
      <button type="button" class="g-link" @click="emit('close')">{{ $t('guest.cancel') }}</button>
    </form>
  </GuestDialog>
</template>
