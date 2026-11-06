<script setup lang="ts">
// 重要日子：好友會在 7 天前收到站內提醒；不重複的日子必須填年份
const MAX = 20
const VIS = ['public', 'friends', 'close', 'private'] as const
const { api } = useApi()
const { t } = useI18n()
const dates = ref<any[]>([])
const f = reactive({ title: '', month: '', day: '', recurring: true, year: '', visibility: 'friends' })
const days = computed(() => daysInMonth(f.month))
const fmt = formatBirthday
const pad = (n: number) => String(n).padStart(2, '0')
const editing = ref<string | null>(null)
const busy = ref(false), err = ref(''), msg = ref('')
async function load() { try { dates.value = (await api('/me/dates')).dates } catch (e: any) { err.value = apiErrMsg(e) } }
onMounted(load)
function edit(d: any) {
  err.value = msg.value = ''; editing.value = d.id
  Object.assign(f, { title: d.title, month: d.month_day.slice(0, 2), day: d.month_day.slice(3, 5), recurring: d.recurring, year: d.year ? String(d.year) : '', visibility: d.visibility })
}
function reset() { editing.value = null; Object.assign(f, { title: '', month: '', day: '', recurring: true, year: '', visibility: 'friends' }) }
async function add() {
  if (busy.value || (!editing.value && dates.value.length >= MAX)) return
  busy.value = true; err.value = msg.value = ''
  try {
    const body = { title: f.title.trim(), month_day: toBirthday(f.month, f.day), year: f.year ? Number(f.year) : null, recurring: f.recurring, visibility: f.visibility }
    if (editing.value) await api(`/me/dates/${editing.value}`, { method: 'PATCH', body })
    else await api('/me/dates', { method: 'POST', body })
    reset()
    msg.value = t('settings.saved'); await load()
  } catch (e: any) { err.value = apiErrMsg(e) } finally { busy.value = false }
}
const removing = ref<any>(null)
async function remove() {
  if (!removing.value || busy.value) return
  busy.value = true; err.value = msg.value = ''
  try { await api(`/me/dates/${removing.value.id}`, { method: 'DELETE' }); removing.value = null; await load() } catch (e: any) { err.value = apiErrMsg(e); removing.value = null } finally { busy.value = false }
}
</script>
<template>
  <section id="dates" class="c-card">
    <h2 class="c-h2">{{ $t('settings.pc.dt.title') }}</h2>
    <p class="c-mute">{{ $t('settings.pc.dt.hint') }}</p>
    <ul class="identity-list">
      <li v-for="d in dates" :key="d.id" class="c-row">
        <span class="c-grow">{{ d.title }}・{{ fmt(d.month_day) }}{{ !d.recurring && d.year ? `・${d.year}` : '' }}・{{ $t(`settings.pc.vis.${d.visibility}`) }}<template v-if="d.past">・{{ $t('settings.pc.dt.past') }}</template></span>
        <button type="button" class="c-btn small" data-edit @click="edit(d)">{{ $t('settings.pc.edit') }}</button>
        <button type="button" class="c-btn small danger" @click="removing = d">{{ $t('settings.pc.remove') }}</button>
      </li>
    </ul>
    <p v-if="!dates.length && !err" class="c-mute">{{ $t('settings.pc.dt.empty') }}</p>
    <form v-if="editing || dates.length < MAX" @submit.prevent="add">
      <label class="c-field"><span>{{ $t('settings.pc.dt.name') }}</span><input v-model="f.title" maxlength="30" required></label>
      <div class="c-field" role="group" aria-labelledby="dt-label"><span id="dt-label">{{ $t('settings.pc.dt.label') }}</span>
        <div class="c-row">
          <select v-model="f.month" required :aria-label="$t('settings.social.monthAria')"><option value="">{{ $t('settings.social.monthOpt') }}</option><option v-for="m in 12" :key="m" :value="pad(m)">{{ $t('settings.social.monthN', { n: m }) }}</option></select>
          <select v-model="f.day" required :aria-label="$t('settings.social.dayAria')"><option value="">{{ $t('settings.social.dayOpt') }}</option><option v-for="d in days" :key="d" :value="pad(d)">{{ $t('settings.social.dayN', { n: d }) }}</option></select>
        </div>
      </div>
      <div class="c-field"><label class="c-switch"><input v-model="f.recurring" type="checkbox" data-recurring><span>{{ $t('settings.pc.dt.recurring') }}</span></label></div>
      <label class="c-field"><span>{{ $t('settings.pc.dt.year') }}</span><input v-model="f.year" type="number" min="1900" max="2100" :required="!f.recurring" data-year></label>
      <small v-if="!f.recurring" class="c-mute">{{ $t('settings.pc.dt.yearHint') }}</small>
      <label class="c-field"><span>{{ $t('settings.pc.showOn') }}</span>
        <select v-model="f.visibility"><option v-for="v in VIS" :key="v" :value="v">{{ $t(`settings.pc.vis.${v}`) }}</option></select>
      </label>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
      <button class="c-btn" :disabled="busy">{{ $t(editing ? 'settings.pc.save' : 'settings.pc.add') }}</button>
      <button v-if="editing" type="button" class="c-btn" data-cancel @click="reset">{{ $t('settings.pc.cancel') }}</button>
    </form>
    <p v-else class="c-mute">{{ $t('settings.pc.dt.max') }}</p>
    <CreatorConfirm :open="!!removing" :title="$t('settings.pc.dt.delTitle', { title: removing?.title })" :text="$t('settings.pc.dt.delText')" :ok="$t('settings.pc.remove')" danger :busy="busy" @close="!busy && (removing = null)" @ok="remove" />
  </section>
</template>
