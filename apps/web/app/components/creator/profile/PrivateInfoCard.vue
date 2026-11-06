<script setup lang="ts">
// 私人資料：年齡、預設收件地址、居住地址。地址只顯示遮罩值，送出後清掉明文（同 ShippingForm）
const VIS = ['private', 'friends', 'close'] as const
const { api } = useApi()
const { t } = useI18n()
const { user } = useAuth()
const birthYear = ref<string>(user.value?.birth_year ? String(user.value.birth_year) : '')
const ageVis = ref(user.value?.age_vis ?? 'private'), homeVis = ref(user.value?.home_address_vis ?? 'private')
const cur = ref<{ shipping: any, home: any }>({ shipping: null, home: null })
const ship = reactive({ recipient_name: '', phone: '', address: '' }), home = ref('')
const st = reactive({ age: { busy: false, err: '', msg: '' }, ship: { busy: false, err: '', msg: '' }, home: { busy: false, err: '', msg: '' } })
const loadAddr = async () => { cur.value = await api('/me/addresses') }
onMounted(() => loadAddr().catch((e: any) => { st.ship.err = apiErrMsg(e) }))
async function run(k: keyof typeof st, fn: () => Promise<unknown>) {
  const s = st[k]
  if (s.busy) return
  s.busy = true; s.err = s.msg = ''
  try { await fn(); s.msg = t('settings.saved') } catch (e: any) { s.err = apiErrMsg(e) } finally { s.busy = false }
}
const saveAge = () => run('age', async () => { user.value = await api('/me', { method: 'PATCH', body: { birth_year: birthYear.value ? Number(birthYear.value) : null, age_vis: ageVis.value } }) })
const saveShip = () => run('ship', async () => {
  await api('/me/addresses/shipping', { method: 'PUT', body: { recipient_name: ship.recipient_name.trim(), phone: ship.phone.trim(), address: ship.address.trim() } })
  Object.assign(ship, { recipient_name: '', phone: '', address: '' }); await loadAddr()
})
const saveHome = () => run('home', async () => {
  if (home.value.trim()) { await api('/me/addresses/home', { method: 'PUT', body: { address: home.value.trim() } }); home.value = '' }
  user.value = await api('/me', { method: 'PATCH', body: { home_address_vis: homeVis.value } }); await loadAddr()
})
const clear = (k: 'shipping' | 'home') => run(k === 'home' ? 'home' : 'ship', async () => { await api(`/me/addresses/${k}`, { method: 'DELETE' }); await loadAddr() })
</script>
<template>
  <section id="private-info" class="c-card">
    <h2 class="c-h2">{{ $t('settings.pc.pv.title') }}</h2>
    <p class="c-mute">{{ $t('settings.pc.pv.hint') }}</p>
    <form @submit.prevent="saveAge">
      <h3 class="c-lbl">{{ $t('settings.pc.pv.age') }}</h3>
      <label class="c-field"><span>{{ $t('settings.pc.pv.birthYear') }}</span><input v-model="birthYear" type="number" min="1900" max="2100" inputmode="numeric"></label>
      <label class="c-field"><span>{{ $t('settings.pc.showOn') }}</span>
        <select v-model="ageVis"><option v-for="v in VIS" :key="v" :value="v">{{ $t(`settings.pc.vis.${v}`) }}</option></select>
      </label>
      <p v-if="st.age.err" class="c-err" role="alert">{{ st.age.err }}</p>
      <p v-if="st.age.msg" class="c-ok" role="status">{{ st.age.msg }}</p>
      <button class="c-btn" :disabled="st.age.busy">{{ $t('settings.save') }}</button>
    </form>
    <form class="c-mt12" @submit.prevent="saveShip">
      <h3 class="c-lbl">{{ $t('settings.pc.pv.shipTitle') }}</h3>
      <dl v-if="cur.shipping" class="c-dl"><dt>{{ $t('settings.pc.pv.recipient') }}</dt><dd>{{ cur.shipping.recipient_name }}</dd><dt>{{ $t('settings.pc.pv.phone') }}</dt><dd>{{ cur.shipping.phone }}</dd><dt>{{ $t('settings.pc.pv.address') }}</dt><dd>{{ cur.shipping.address }}</dd></dl>
      <p v-else class="c-mute">{{ $t('settings.pc.pv.noneSaved') }}</p>
      <label class="c-field"><span>{{ $t('settings.pc.pv.recipient') }}</span><input v-model="ship.recipient_name" maxlength="50" required autocomplete="name"></label>
      <label class="c-field"><span>{{ $t('settings.pc.pv.phone') }}</span><input v-model="ship.phone" type="tel" maxlength="20" minlength="6" required autocomplete="tel"></label>
      <label class="c-field"><span>{{ $t('settings.pc.pv.address') }}</span><input v-model="ship.address" maxlength="200" minlength="5" required autocomplete="street-address"></label>
      <p v-if="cur.shipping" class="c-mute c-small">{{ $t('settings.pc.pv.keepHint') }}</p>
      <p v-if="st.ship.err" class="c-err" role="alert">{{ st.ship.err }}</p>
      <p v-if="st.ship.msg" class="c-ok" role="status">{{ st.ship.msg }}</p>
      <div class="c-row"><button class="c-btn" :disabled="st.ship.busy">{{ $t('settings.save') }}</button><button v-if="cur.shipping" type="button" class="c-btn danger" :disabled="st.ship.busy" @click="clear('shipping')">{{ $t('settings.pc.pv.clear') }}</button></div>
    </form>
    <form class="c-mt12" @submit.prevent="saveHome">
      <h3 class="c-lbl">{{ $t('settings.pc.pv.homeTitle') }}</h3>
      <p class="c-mute">{{ cur.home ? cur.home.address : $t('settings.pc.pv.noneSaved') }}</p>
      <label class="c-field"><span>{{ $t('settings.pc.pv.address') }}</span><input v-model="home" maxlength="200" minlength="5" autocomplete="street-address"></label>
      <label class="c-field"><span>{{ $t('settings.pc.showOn') }}</span>
        <select v-model="homeVis"><option v-for="v in VIS" :key="v" :value="v">{{ $t(`settings.pc.vis.${v}`) }}</option></select>
      </label>
      <p v-if="st.home.err" class="c-err" role="alert">{{ st.home.err }}</p>
      <p v-if="st.home.msg" class="c-ok" role="status">{{ st.home.msg }}</p>
      <div class="c-row"><button class="c-btn" :disabled="st.home.busy">{{ $t('settings.save') }}</button><button v-if="cur.home" type="button" class="c-btn danger" :disabled="st.home.busy" @click="clear('home')">{{ $t('settings.pc.pv.clear') }}</button></div>
    </form>
  </section>
</template>
