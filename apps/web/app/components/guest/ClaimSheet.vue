<script setup lang="ts">
import '~/assets/guest.css'
// 捐助 / 修改捐助 bottom sheet（docs 03 §5.7）。Idempotency-Key 於開啟時產生，重試沿用。
const props = defineProps<{ item: any, mine: any | null, online: boolean, loggedInName?: string }>()
const emit = defineEmits<{ close: [], done: [r: any], stale: [remaining: number], switchEdit: [], login: [] }>()
const { api, newKey } = useGuest()

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
const busy = ref(false)
const msg = ref('')
const errs = ref<Record<string, string>>({})
const note0 = note.value
const dirty = computed(() => note.value !== note0)

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
        body: { qty: qty.value, note: note.value || undefined, visibility: vis.value },
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
      msg.value = rem > 0 ? `抱歉，只剩 ${rem} 個了。` : '剛剛被別人捐助完了。'
      if (rem <= 0) setTimeout(() => emit('close'), 1200)
    } else if (c === 'CLAIM_ALREADY_EXISTS') {
      msg.value = '你已經捐助過這個品項，已幫你切換到修改'
      emit('switchEdit')
    } else if (c === 'VALIDATION_FAILED') {
      for (const x of e.data.errors ?? []) errs.value[x.pointer.slice(1)] = x.detail
      if (!Object.keys(errs.value).length) msg.value = e.detail // 欄位下已有錯誤就不在橫幅重複
    } else if (c === 'IDEMPOTENCY_CONFLICT') msg.value = '請求重複，請關閉後重新操作'
    else msg.value = e.detail
  } finally { busy.value = false }
}
</script>

<template>
  <GuestDialog labelledby="claim-title" :dirty="dirty" @close="emit('close')">
    <form class="g-sheet" @submit.prevent="submit">
      <div class="g-item">
        <img v-if="item.image_url" class="g-thumb" :src="item.image_url" alt="">
        <div class="g-body"><div id="claim-title" class="g-title">{{ editing ? '修改捐助' : '捐助' }}「{{ item.title }}」</div>
          <div class="g-mute">還缺 {{ item.qty_remaining }} 個，你要送幾個？</div></div>
      </div>
      <div class="g-step">
        <button type="button" aria-label="減少" :disabled="busy || qty <= 1" @click="qty--">－</button>
        <b>{{ qty }}</b>
        <button type="button" aria-label="增加" :disabled="busy || qty >= max" @click="qty++">＋</button>
      </div>
      <div v-if="msg" class="g-banner err" role="alert">{{ msg }}</div>
      <template v-if="!editing">
        <p class="g-mute">以 {{ loggedInName }} 的身分捐助</p>
        <template v-if="vis">
          <label for="cv">這筆捐助誰看得到</label>
          <select id="cv" v-model="vis" :disabled="busy"><option value="public">公開</option><option value="friends">僅好友</option><option value="private">私人</option></select>
        </template>
      </template>
      <label for="cm">留言（選填）</label>
      <input id="cm" v-model="note" maxlength="200" :readonly="busy">
      <div v-if="errs.note" class="g-field-err">{{ errs.note }}</div>
      <p class="g-mute">認捐需登入會員。捐助後可隨時修改或取消。我們如何使用你的資料：<NuxtLink to="/privacy#collect" target="_blank">個資蒐集告知</NuxtLink><span class="g-nowrap">（不需勾選）</span></p>
      <button class="g-btn" :disabled="busy || !online">{{ !online ? '離線中，暫時無法捐助' : busy ? '送出中…' : editing ? '儲存修改' : '確認捐助' }}</button>
      <button type="button" class="g-link" @click="emit('close')">取消</button>
    </form>
  </GuestDialog>
</template>
