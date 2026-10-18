<script setup lang="ts">
// 清單收件資訊（眾籌達標後平台代購寄送用）：只顯示遮罩後的值，要修改需重新輸入完整內容。
const props = defineProps<{ wishlistId: string }>()
const emit = defineEmits<{ saved: [r: any] }>()
const { api } = useApi()
const { t } = useI18n()
const cur = ref<any>(null)
const f = reactive({ recipient_name: '', phone: '', address: '' })
const busy = ref(false), err = ref(''), msg = ref('')
const errs = ref<Record<string, string>>({})
onMounted(async () => {
  try { cur.value = await api(`/wishlists/${props.wishlistId}/shipping-address`); emit('saved', cur.value) } catch (e) { err.value = errMsg(e) }
})
async function save() {
  err.value = msg.value = ''; errs.value = {}
  busy.value = true
  try {
    cur.value = await api(`/wishlists/${props.wishlistId}/shipping-address`, { method: 'PUT', body: { recipient_name: f.recipient_name.trim(), phone: f.phone.trim(), address: f.address.trim() } })
    Object.assign(f, { recipient_name: '', phone: '', address: '' }) // 明文不留在畫面上
    msg.value = t('creator.shipping.saved'); emit('saved', cur.value)
  } catch (e: any) {
    for (const x of e.errors ?? []) errs.value[String(x.pointer ?? '').slice(1)] = x.detail
    err.value = Object.keys(errs.value).length ? '' : errMsg(e)
  } finally { busy.value = false }
}
</script>
<template>
  <div class="c-mt12">
    <p class="c-mute">{{ $t('creator.shipping.intro') }}</p>
    <dl v-if="cur?.has_shipping_address" class="c-dl"><dt>{{ $t('creator.shipping.recipient') }}</dt><dd>{{ cur.recipient_name }}</dd><dt>{{ $t('creator.shipping.phone') }}</dt><dd>{{ cur.phone }}</dd><dt>{{ $t('creator.shipping.address') }}</dt><dd>{{ cur.address }}</dd></dl>
    <p v-else-if="cur" class="c-err">{{ $t('creator.shipping.empty') }}</p>
    <form @submit.prevent="save">
      <label class="c-field"><span>{{ $t('creator.shipping.recipientLabel') }}</span><input v-model="f.recipient_name" maxlength="50" required autocomplete="name"><small v-if="errs.recipient_name" class="c-err">{{ errs.recipient_name }}</small></label>
      <label class="c-field"><span>{{ $t('creator.shipping.phoneLabel') }}</span><input v-model="f.phone" type="tel" maxlength="20" minlength="6" required autocomplete="tel"><small v-if="errs.phone" class="c-err">{{ errs.phone }}</small></label>
      <label class="c-field"><span>{{ $t('creator.shipping.addressLabel') }}</span><input v-model="f.address" maxlength="200" minlength="5" required autocomplete="street-address"><small v-if="errs.address" class="c-err">{{ errs.address }}</small></label>
      <p v-if="cur?.has_shipping_address" class="c-mute c-small">{{ $t('creator.shipping.privacy') }}</p>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
      <button class="c-btn block" :disabled="busy">{{ busy ? $t('creator.shipping.saving') : cur?.has_shipping_address ? $t('creator.shipping.update') : $t('creator.shipping.save') }}</button>
    </form>
  </div>
</template>
