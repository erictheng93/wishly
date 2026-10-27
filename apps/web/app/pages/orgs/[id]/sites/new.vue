<script setup lang="ts">
// 建立據點：選事件與有效的勸募許可，成功後進入清單編輯頁加品項
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('org.relief.stNewTitle') })
const { api } = useApi()
const id = useRoute().params.id as string
const f = reactive({ title: '', description: '', event_id: (useRoute().query.event as string) || '', permit_id: '', site_address: '', recipient_name: '', recipient_phone: '', claim_ttl_hours: 48 as number | null })
const events = ref<any[]>([]), permits = ref<any[]>([])
const busy = ref(false), err = ref('')
onMounted(async () => {
  try {
    const [ev, pm] = await Promise.all([api(`/orgs/${id}/events`), api(`/orgs/${id}/permits`)])
    events.value = ev
    const today = taipeiToday()
    permits.value = pm.filter((p: any) => p.status === 'approved' && p.valid_to >= today)
    if (permits.value.length === 1) f.permit_id = permits.value[0].id
  } catch (e) { err.value = errMsg(e) }
})
async function submit() {
  err.value = ''; busy.value = true
  try {
    const r = await api(`/orgs/${id}/sites`, { method: 'POST', body: { ...f, title: f.title.trim(), description: f.description || undefined, claim_ttl_hours: f.claim_ttl_hours ?? undefined } })
    await navigateTo(`/lists/${(r.wishlist ?? r).id}`)
  } catch (e: any) {
    err.value = e.code === 'ORG_NOT_VERIFIED' ? t('org.relief.notVerified') : errMsg(e)
  } finally { busy.value = false }
}
</script>

<template>
  <main class="c-page">
    <CreatorHeader :title="$t('org.relief.stNewTitle')" :back="`/orgs/${id}`" />
    <form class="c-card" @submit.prevent="submit">
      <label class="c-field"><span>{{ $t('org.relief.stEvent') }}</span>
        <select v-model="f.event_id" name="event_id" required><option value="" disabled>{{ $t('org.relief.stEventPick') }}</option><option v-for="e in events" :key="e.id" :value="e.id">{{ e.name }}（{{ e.occurred_on }}）</option></select>
        <NuxtLink :to="`/orgs/${id}/events/new`">{{ $t('org.relief.stNewEvent') }}</NuxtLink>
      </label>
      <label class="c-field"><span>{{ $t('org.relief.stPermit') }}</span>
        <select v-model="f.permit_id" name="permit_id" required><option value="" disabled>{{ $t('org.relief.stPermitPick') }}</option><option v-for="p in permits" :key="p.id" :value="p.id">{{ p.permit_no }}（～{{ p.valid_to }}）</option></select>
        <small v-if="!permits.length" class="c-err">{{ $t('org.relief.stNoPermit') }}</small>
      </label>
      <label class="c-field"><span>{{ $t('org.relief.stTitle') }}</span><input v-model="f.title" name="title" maxlength="100" required></label>
      <label class="c-field"><span>{{ $t('org.relief.stDesc') }}</span><textarea v-model="f.description" rows="2" /></label>
      <label class="c-field"><span>{{ $t('org.relief.stAddr') }}</span><input v-model="f.site_address" name="site_address" maxlength="200" required></label>
      <label class="c-field"><span>{{ $t('org.relief.stRecipient') }}</span><input v-model="f.recipient_name" name="recipient_name" maxlength="50" required></label>
      <label class="c-field"><span>{{ $t('org.relief.stPhone') }}</span><input v-model="f.recipient_phone" name="recipient_phone" type="tel" maxlength="30" required></label>
      <label class="c-field"><span>{{ $t('org.relief.stTtl') }}</span><input v-model.number="f.claim_ttl_hours" type="number" inputmode="numeric" min="1" max="720"></label>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <button class="c-btn primary block" :disabled="busy">{{ $t('org.relief.stSubmit') }}</button>
    </form>
  </main>
</template>
