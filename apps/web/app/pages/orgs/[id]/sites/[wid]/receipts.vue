<script setup lang="ts">
// 據點實收登記：任何成員（含 volunteer）可看品項與累計實收並登記；GET/POST 皆走 /orgs/{org}/...
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('org.console.receipts') })
const { api } = useApi()
const route = useRoute()
const org = route.params.id as string, wid = route.params.wid as string
const data = ref<any>(null), loading = ref(true), err = ref('')
const rcpt = ref<any>(null), rcptKey = ref(''), rcptF = reactive({ qty: 1, note: '' }), rcptBusy = ref(false), rcptErr = ref('')
async function load() {
  try { data.value = await api(`/orgs/${org}/sites/${wid}/receipts`) } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
onMounted(load)
function openReceipt(i: any) { rcptKey.value = crypto.randomUUID(); rcpt.value = i; rcptF.qty = 1; rcptF.note = ''; rcptErr.value = '' }
async function saveReceipt() {
  rcptBusy.value = true; rcptErr.value = ''
  try {
    await api(`/orgs/${org}/receipts`, { method: 'POST', headers: { 'Idempotency-Key': rcptKey.value }, noRedirect: true, body: { item_id: rcpt.value.item_id, qty: Number(rcptF.qty), note: rcptF.note || undefined } })
    rcpt.value = null; await load()
  } catch (e: any) { rcptErr.value = errMsg(e); if (e.code !== 'NETWORK') rcptKey.value = crypto.randomUUID() } finally { rcptBusy.value = false }
}
</script>

<template>
  <main class="c-page">
    <CreatorHeader :title="data?.site?.title || $t('org.console.receipts')" :back="`/orgs/${org}`" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="loading" class="c-mute" role="status">{{ $t('org.apply.loading') }}</p>
    <template v-else-if="data">
      <p class="c-mute"><CreatorStatusBadge v-if="data.site.site_status" :v="data.site.site_status" />　{{ data.site.site_address }}</p>
      <p v-if="!data.items.length" class="c-mute">{{ $t('org.relief.receiptsNone') }}</p>
      <ul class="c-list">
        <li v-for="i in data.items" :key="i.item_id" class="c-card">
          <strong>{{ i.title }}</strong>
          <span class="c-mute">{{ $t(`org.relief.category.${i.category}`) }}・{{ $t(`org.relief.urgency.${i.urgency}`) }}</span>
          <span class="c-mute">{{ $t('org.relief.receiptNeeded', { n: i.qty_needed }) }}　{{ $t('org.relief.receiptTotal', { n: i.total }) }}</span>
          <button class="c-btn" :disabled="data.site.site_status === 'closed'" @click="openReceipt(i)">{{ $t('org.relief.receiptBtn') }}</button>
        </li>
      </ul>
    </template>

    <CreatorSheet :open="!!rcpt" :title="$t('org.relief.receiptTitle')" @close="rcpt = null">
      <form v-if="rcpt" @submit.prevent="saveReceipt">
        <p><strong>{{ rcpt.title }}</strong><span class="c-mute">　{{ $t('org.relief.receiptTotal', { n: rcpt.total }) }}</span></p>
        <ul class="c-mute"><li v-for="(e, k) in rcpt.entries" :key="k">+{{ e.qty }} {{ e.note }} {{ $t('org.relief.receiptBy', { name: e.recorded_by_name, time: fmtTime(e.created_at) }) }}</li></ul>
        <label class="c-field"><span>{{ $t('org.relief.receiptQty') }}</span><input v-model="rcptF.qty" name="qty" type="number" inputmode="numeric" min="1" required></label>
        <label class="c-field"><span>{{ $t('org.relief.receiptNote') }}</span><input v-model="rcptF.note" name="note" maxlength="200"></label>
        <p v-if="rcptErr" class="c-err" role="alert">{{ rcptErr }}</p>
        <button class="c-btn primary block" :disabled="rcptBusy">{{ $t('org.relief.receiptSave') }}</button>
      </form>
    </CreatorSheet>
  </main>
</template>

<style scoped>
.c-list{list-style:none;padding:0;margin:12px 0 0;display:grid;gap:12px}
.c-list li{display:flex;flex-wrap:wrap;gap:4px 12px;align-items:baseline}
</style>
