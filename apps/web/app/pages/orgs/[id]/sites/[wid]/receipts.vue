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
  <main class="c-page rc">
    <CreatorHeader :title="data?.site?.title || $t('org.console.receipts')" :back="`/orgs/${org}`" />
    <p v-if="err" class="c-err c-errbox" role="alert">{{ err }}</p>
    <p v-if="loading" class="c-mute" role="status">{{ $t('org.apply.loading') }}</p>
    <template v-else-if="data">
      <p class="c-mute rc-site"><CreatorStatusBadge v-if="data.site.site_status" :v="data.site.site_status" />　{{ data.site.site_address }}</p>
      <p v-if="!data.items.length" class="c-mute">{{ $t('org.relief.receiptsNone') }}</p>
      <ul class="c-list rc-list">
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
        <p v-if="rcptErr" class="c-err c-errbox" role="alert">{{ rcptErr }}</p>
        <button class="c-btn primary block" :disabled="rcptBusy">{{ $t('org.relief.receiptSave') }}</button>
      </form>
    </CreatorSheet>
  </main>
</template>

<style scoped>
.rc-site{display:flex;align-items:center;gap:12px;margin:0 0 20px;font:500 13px var(--mono)}
.rc-list{list-style:none;padding:0;margin:0!important;display:block;counter-reset:rc;border-top:2px solid var(--ink-line)}
.rc-list li.c-card{counter-increment:rc;display:grid;grid-template-columns:auto 1fr auto;gap:2px 16px;align-items:center;margin:0;padding:16px 4px 16px 0;border:0;border-bottom:1px solid var(--line);animation:rise .5s var(--ease) both}
.rc-list li.c-card::before{content:counter(rc,decimal-leading-zero) / "";grid-row:1/4;font:700 22px var(--mono);color:var(--red-text);letter-spacing:-.04em;min-width:1.8em;padding-left:2px}
.rc-list li>strong{grid-column:2;font-size:17px}
.rc-list li>.c-mute{grid-column:2;font:500 12px/1.5 var(--mono);letter-spacing:.03em}
.rc-list li>.c-btn{grid-column:3;grid-row:1/4}
@media (max-width:520px){.rc-list li>.c-btn{grid-column:2;grid-row:auto;justify-self:start;margin-top:8px}}
</style>
