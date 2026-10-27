<script setup lang="ts">
// 建立救災事件：先依縣市＋日期查重，使用者確認都不是才顯示建立表單
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('org.relief.evNewTitle') })
const { api } = useApi()
const id = useRoute().params.id as string
const f = reactive({ name: '', hazard_type: 'earthquake', county: '', counties: [] as string[], occurred_on: '', area_note: '', description: '' })
const similar = ref<any[]>([]), dismissed = ref(false), checked = ref(false), busy = ref(false), err = ref('')
// 查重看主要縣市＋日期（選了會自動勾進受災縣市）；回應過期（使用者又改了條件）就丟棄
let seq = 0
watch(() => [f.county, f.occurred_on], async ([county, date]) => {
  const n = ++seq
  similar.value = []; dismissed.value = false; checked.value = false
  if (county && !f.counties.includes(county)) f.counties.push(county)
  if (!county || !date) return
  try {
    const r = await api('/relief-events/similar', { query: { county, date } })
    if (n === seq) { similar.value = r; checked.value = true }
  } catch { if (n === seq) checked.value = true /* 查重失敗不擋建立 */ }
})
const showForm = computed(() => (checked.value && !similar.value.length) || dismissed.value)
async function submit() {
  err.value = ''
  if (!f.counties.length) { err.value = t('org.relief.evCounties'); return }
  busy.value = true
  try {
    await api(`/orgs/${id}/events`, { method: 'POST', body: { name: f.name.trim(), hazard_type: f.hazard_type, counties: f.counties, occurred_on: f.occurred_on, area_note: f.area_note || undefined, description: f.description || undefined } })
    await navigateTo(`/orgs/${id}`)
  } catch (e: any) {
    err.value = e.code === 'ORG_NOT_VERIFIED' ? t('org.relief.notVerified') : errMsg(e)
  } finally { busy.value = false }
}
</script>

<template>
  <main class="c-page">
    <CreatorHeader :title="$t('org.relief.evNewTitle')" :back="`/orgs/${id}`" />
    <div class="c-card">
      <div class="c-row wrap">
        <label class="c-field c-grow"><span>{{ $t('org.relief.evCounties') }}</span>
          <select v-model="f.county" name="county"><option value="" disabled>{{ $t('org.relief.pick') }}</option><option v-for="c in COUNTIES" :key="c" :value="c">{{ c }}</option></select>
        </label>
        <label class="c-field c-grow"><span>{{ $t('org.relief.evDate') }}</span><input v-model="f.occurred_on" type="date" name="occurred_on"></label>
      </div>
    </div>

    <section v-if="similar.length && !dismissed" class="c-card">
      <h2 class="c-h2">{{ $t('org.relief.similarTitle') }}</h2>
      <p class="c-mute">{{ $t('org.relief.similarHint') }}</p>
      <article v-for="e in similar" :key="e.id" class="c-card">
        <strong>{{ e.name }}</strong>
        <div class="c-mute">{{ e.occurred_on }} · {{ (e.counties ?? []).join(' / ') }}</div>
        <NuxtLink :to="`/orgs/${id}/sites/new?event=${e.id}`" class="c-btn primary c-mt">{{ $t('org.relief.useThis') }}</NuxtLink>
      </article>
      <button type="button" class="c-btn block" @click="dismissed = true">{{ $t('org.relief.noneOfThese') }}</button>
    </section>

    <form v-if="showForm && f.county && f.occurred_on" class="c-card" @submit.prevent="submit">
      <label class="c-field"><span>{{ $t('org.relief.evName') }}</span><input v-model="f.name" name="name" maxlength="100" required></label>
      <label class="c-field"><span>{{ $t('org.relief.evHazard') }}</span>
        <select v-model="f.hazard_type" name="hazard_type"><option v-for="h in HAZARDS" :key="h" :value="h">{{ $t(`org.relief.hazard.${h}`) }}</option></select>
      </label>
      <fieldset class="c-field"><legend>{{ $t('org.relief.evCounties') }}</legend>
        <label v-for="c in COUNTIES" :key="c" class="c-switch"><input v-model="f.counties" type="checkbox" :value="c"><span>{{ c }}</span></label>
      </fieldset>
      <label class="c-field"><span>{{ $t('org.relief.evArea') }}</span><input v-model="f.area_note" maxlength="200"></label>
      <label class="c-field"><span>{{ $t('org.relief.evDesc') }}</span><textarea v-model="f.description" rows="3" /></label>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <button class="c-btn primary block" :disabled="busy">{{ busy ? $t('org.relief.evSubmitting') : $t('org.relief.evSubmit') }}</button>
    </form>
  </main>
</template>
