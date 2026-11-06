<script setup lang="ts">
// 重要日子：好友會在 7 天前收到站內提醒；不重複的日子必須填年份
const MAX = 20
const VIS = ['public', 'friends', 'close', 'private'] as const
const { api } = useApi()
const { t } = useI18n()
const dates = ref<any[]>([])
const f = reactive({ title: '', month: '', day: '', recurring: true, year: '', visibility: 'friends' })
const days = computed(() => daysInMonth(f.month))
const pad = (n: number) => String(n).padStart(2, '0')
const busy = ref(false), err = ref(''), msg = ref('')
async function load() { try { dates.value = (await api('/me/dates')).dates } catch (e: any) { err.value = apiErrMsg(e) } }
onMounted(load)
async function add() {
  if (busy.value || dates.value.length >= MAX) return
  busy.value = true; err.value = msg.value = ''
  try {
    await api('/me/dates', { method: 'POST', body: { title: f.title.trim(), month_day: toBirthday(f.month, f.day), year: f.year ? Number(f.year) : null, recurring: f.recurring, visibility: f.visibility } })
    Object.assign(f, { title: '', month: '', day: '', recurring: true, year: '' })
    msg.value = t('settings.saved'); await load()
  } catch (e: any) { err.value = apiErrMsg(e) } finally { busy.value = false }
}
async function remove(id: string) {
  err.value = msg.value = ''
  try { await api(`/me/dates/${id}`, { method: 'DELETE' }); await load() } catch (e: any) { err.value = apiErrMsg(e) }
}
</script>
<template>
  <section id="dates" class="c-card">
    <h2 class="c-h2">{{ $t('settings.pc.dt.title') }}</h2>
    <p class="c-mute">{{ $t('settings.pc.dt.hint') }}</p>
    <ul class="identity-list">
      <li v-for="d in dates" :key="d.id" class="c-row">
        <span class="c-grow">{{ d.title }}・{{ d.month_day }}{{ d.year ? `・${d.year}` : '' }}・{{ $t(`settings.pc.vis.${d.visibility}`) }}<template v-if="d.past">・{{ $t('settings.pc.dt.past') }}</template></span>
        <button type="button" class="c-btn small danger" @click="remove(d.id)">{{ $t('settings.pc.remove') }}</button>
      </li>
    </ul>
    <p v-if="!dates.length && !err" class="c-mute">{{ $t('settings.pc.dt.empty') }}</p>
    <form v-if="dates.length < MAX" @submit.prevent="add">
      <label class="c-field"><span>{{ $t('settings.pc.dt.name') }}</span><input v-model="f.title" maxlength="30" required></label>
      <div class="c-row">
        <select v-model="f.month" required :aria-label="$t('settings.pc.dt.month')"><option value="">{{ $t('settings.pc.dt.month') }}</option><option v-for="m in 12" :key="m" :value="pad(m)">{{ m }}</option></select>
        <select v-model="f.day" required :aria-label="$t('settings.pc.dt.day')"><option value="">{{ $t('settings.pc.dt.day') }}</option><option v-for="d in days" :key="d" :value="pad(d)">{{ d }}</option></select>
      </div>
      <label class="c-field c-row"><input v-model="f.recurring" type="checkbox" data-recurring>{{ $t('settings.pc.dt.recurring') }}</label>
      <label class="c-field"><span>{{ $t('settings.pc.dt.year') }}</span><input v-model="f.year" type="number" min="1900" max="2100" :required="!f.recurring" data-year></label>
      <small v-if="!f.recurring" class="c-mute">{{ $t('settings.pc.dt.yearHint') }}</small>
      <label class="c-field"><span>{{ $t('settings.pc.showOn') }}</span>
        <select v-model="f.visibility"><option v-for="v in VIS" :key="v" :value="v">{{ $t(`settings.pc.vis.${v}`) }}</option></select>
      </label>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
      <button class="c-btn" :disabled="busy">{{ $t('settings.pc.add') }}</button>
    </form>
    <p v-else class="c-mute">{{ $t('settings.pc.dt.max') }}</p>
  </section>
</template>
