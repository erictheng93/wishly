<script setup lang="ts">
// 組織申請表（4 步，草稿自動保存）與審核狀態；同一頁依 verification_status 切換
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('org.apply.title') })
const { api } = useApi()
const { user, fetchMe } = useAuth()
const { switchTo } = useOrgMode()

const org = ref<any>(null)
const form = reactive<Record<string, string>>({})
const step = ref(1)
const loading = ref(true), busy = ref(false), err = ref(''), agree = ref(false)
const permits = ref<any[]>([])
const permit = reactive({ permit_no: '', valid_from: '', valid_to: '' })
const permitFile = ref<File | null>(null)
const taxErr = ref('')
const FIELDS = ['legal_name', 'display_name', 'org_type', 'tax_id', 'registration_no', 'authority', 'website', 'intro',
  'rep_name', 'contact_name', 'contact_title', 'contact_phone', 'public_email', 'public_phone']
const STEPS: string[][] = [FIELDS.slice(0, 8), FIELDS.slice(8)]
// 驗證錯誤的 pointer → 回到哪一步
const ptrStep = (p: string) => (p === '/documents' || p === '/permits' ? 3 : STEPS[1]!.includes(p.slice(1)) ? 2 : 1)
const REQUIRED = new Set(['legal_name', 'display_name', 'org_type', 'tax_id', 'registration_no', 'authority', 'rep_name', 'contact_name', 'contact_phone', 'public_email'])
const status = computed(() => org.value?.verification_status)
const wizard = computed(() => !org.value || status.value === 'draft' || status.value === 'needs_changes')
const taxBad = computed(() => !!form.tax_id && !taxIdOk(form.tax_id))
const can = (f: string) => !org.value || editable(org.value, f)
const docs = (kind: string) => (org.value?.documents ?? []).filter((d: any) => d.kind === kind)
const stepOk = computed(() => (STEPS[step.value - 1] ?? []).every(f => !REQUIRED.has(f) || form[f]) && (step.value !== 1 || !taxBad.value))

// first＝初次載入才決定從第幾步繼續；之後重載（上傳、送出）不動目前步驟
async function reload(first = false) {
  const o = await api(`/orgs/${org.value.id}`)
  org.value = o
  for (const f of FIELDS) form[f] = o[f] ?? ''
  await loadPermits()
  if (first) step.value = o.verification_status === 'needs_changes' ? 1 : stepOf(o, permits.value)
}
async function loadPermits() { permits.value = await api(`/orgs/${org.value.id}/permits`) }
async function run(fn: () => Promise<void>) {
  busy.value = true; err.value = ''
  try { await fn() } catch (e: any) {
    err.value = e?.status === 413 ? t('org.apply.fileBig') : errMsg(e)
    if (e?.code === 'TAX_ID_TAKEN') { step.value = 1; taxErr.value = err.value }
    const p = e?.errors?.[0]?.pointer
    if (e?.code === 'VALIDATION_FAILED' && p) step.value = ptrStep(p)
    if (e?.code === 'ORG_DRAFT_EXISTS') await init()
  } finally { busy.value = false }
}
async function init() {
  await fetchMe(true)
  const mine = (user.value?.orgs ?? []).find((o: any) => ['draft', 'needs_changes', 'pending', 'rejected', 'verified'].includes(o.verification_status) && o.role === 'owner')
  if (mine) { org.value = { id: mine.id }; await reload(true) }
  loading.value = false
}
onMounted(() => run(init).finally(() => { loading.value = false }))

// 欄位 blur 自動保存；尚無草稿時等第 1 步「下一步」才建立
async function save(f: string) {
  if (!org.value || !can(f)) return
  await run(async () => { await api(`/orgs/${org.value.id}`, { method: 'PATCH', body: { [f]: form[f] } }) })
}
async function next() {
  await run(async () => {
    if (!org.value) {
      const body = Object.fromEntries(FIELDS.filter(f => form[f]).map(f => [f, form[f]]))
      org.value = await api('/orgs', { method: 'POST', body })
      await reload()
    }
    step.value++
  })
}
async function upload(kind: 'registration' | 'permit', file: File | undefined) {
  if (!file) return null
  if (file.size > 10 * 1024 * 1024) { err.value = t('org.apply.fileBig'); return null }
  if (!['application/pdf', 'image/jpeg', 'image/png'].includes(file.type)) { err.value = t('org.apply.fileType'); return null }
  // File 不會被 ofetch 序列化；走 api() 才有錯誤正規化與 401 導向
  return await api<any>(`/orgs/${org.value.id}/documents`, { method: 'POST', query: { kind }, body: file, headers: { 'Content-Type': file.type } })
}
const onFile = (kind: 'registration' | 'permit', e: Event) => {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  if (kind === 'permit') { permitFile.value = file ?? null; return }
  run(async () => { await upload(kind, file); await reload() }).finally(() => { input.value = '' })
}
const delDoc = (id: string) => run(async () => { await api(`/orgs/${org.value.id}/documents/${id}`, { method: 'DELETE' }); await reload() })
const addPermit = () => run(async () => {
  const d = await upload('permit', permitFile.value ?? undefined)
  if (!d) return
  try { await api(`/orgs/${org.value.id}/permits`, { method: 'POST', body: { ...permit, document_id: d.id } }) } catch (e) {
    await api(`/orgs/${org.value.id}/documents/${d.id}`, { method: 'DELETE' }).catch(() => {}) // 不留孤兒文件
    throw e
  }
  Object.assign(permit, { permit_no: '', valid_from: '', valid_to: '' }); permitFile.value = null
  await reload()
})
const canReapply = computed(() => !org.value?.resubmit_after || Date.now() >= Date.parse(org.value.resubmit_after))
const reapply = () => run(async () => { await api(`/orgs/${org.value.id}`, { method: 'PATCH', body: { legal_name: org.value.legal_name } }); await reload(true) })
const submit = () => run(async () => { await api(`/orgs/${org.value.id}/submit`, { method: 'POST' }); await reload(); await fetchMe(true) })
const withdraw = () => run(async () => { await api(`/orgs/${org.value.id}/withdraw`, { method: 'POST' }); await reload(); await fetchMe(true) })
</script>

<template>
  <main class="c-page">
    <CreatorHeader :title="$t('org.apply.title')" back="/org" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="loading" class="c-mute">{{ $t('org.apply.loading') }}</p>

    <template v-else-if="wizard">
      <p v-if="status === 'needs_changes'" class="c-card" role="status"><strong>{{ $t('org.apply.needsChanges') }}</strong><br>{{ org.review_note }}</p>
      <p class="c-mute">{{ $t('org.apply.stepOf', { n: step }) }}　{{ [1, 2, 3, 4].map(n => $t(`org.apply.s${n}`)).join(' / ') }}</p>

      <section v-for="(fs, i) in STEPS" v-show="step === i + 1" :key="i" class="c-card">
        <h2 class="c-h2">{{ $t(`org.apply.s${i + 1}`) }}</h2>
        <label v-for="f in fs" :key="f" class="c-field">
          <span>{{ $t(`org.apply.f.${f}`) }}<template v-if="!REQUIRED.has(f)">（{{ $t('org.apply.optional') }}）</template></span>
          <select v-if="f === 'org_type'" v-model="form[f]" :disabled="!can(f)" @change="save(f)">
            <option value="" disabled />
            <option v-for="ty in ORG_TYPES" :key="ty" :value="ty">{{ $t(`org.apply.type.${ty}`) }}</option>
          </select>
          <textarea v-else-if="f === 'intro'" v-model="form[f]" rows="4" :disabled="!can(f)" @blur="save(f)" />
          <input v-else v-model="form[f]" :inputmode="f === 'tax_id' ? 'numeric' : undefined" :maxlength="f === 'tax_id' ? 8 : undefined" :disabled="!can(f)" @blur="save(f)" @input="taxErr = ''">
          <small v-if="f === 'tax_id' && taxBad" class="c-err">{{ $t('org.apply.taxBad') }}</small>
          <small v-else-if="f === 'tax_id' && taxErr" class="c-err">{{ taxErr }}</small>
        </label>
      </section>

      <section v-show="step === 3" class="c-card">
        <h2 class="c-h2">{{ $t('org.apply.s3') }}</h2>
        <p class="c-mute">{{ $t('org.apply.fileHint') }}</p>
        <h3>{{ $t('org.apply.regDoc') }}</h3>
        <ul><li v-for="d in docs('registration')" :key="d.id">{{ d.content_type }} · {{ Math.ceil(d.size_bytes / 1024) }} KB
          <button type="button" class="c-btn" @click="delDoc(d.id)">{{ $t('org.apply.remove') }}</button></li></ul>
        <input type="file" accept="application/pdf,image/jpeg,image/png" :disabled="busy" @change="onFile('registration', $event)">
        <h3>{{ $t('org.apply.permitDoc') }}</h3>
        <ul><li v-for="p in permits" :key="p.id">{{ p.permit_no }}　{{ p.valid_from }} ~ {{ p.valid_to }}</li></ul>
        <label class="c-field"><span>{{ $t('org.apply.permitNo') }}</span><input v-model="permit.permit_no"></label>
        <label class="c-field"><span>{{ $t('org.apply.validFrom') }}</span><input v-model="permit.valid_from" type="date"></label>
        <label class="c-field"><span>{{ $t('org.apply.validTo') }}</span><input v-model="permit.valid_to" type="date"></label>
        <input type="file" accept="application/pdf,image/jpeg,image/png" :disabled="busy" @change="onFile('permit', $event)">
        <button type="button" class="c-btn" :disabled="busy || !permit.permit_no || !permit.valid_from || !permit.valid_to || !permitFile" @click="addPermit">{{ $t('org.apply.addPermit') }}</button>
      </section>

      <section v-show="step === 4" class="c-card">
        <h2 class="c-h2">{{ $t('org.apply.s4') }}</h2>
        <dl><template v-for="f in FIELDS" :key="f"><template v-if="form[f]"><dt class="c-mute">{{ $t(`org.apply.f.${f}`) }}</dt><dd>{{ form[f] }}</dd></template></template></dl>
        <label><input v-model="agree" type="checkbox"> {{ $t('org.apply.agree') }}</label>
        <p class="c-mute">{{ $t('org.apply.reviewEta') }}</p>
        <button type="button" class="c-btn" :disabled="busy || !agree" @click="submit">{{ $t('org.apply.submit') }}</button>
      </section>

      <div class="c-row">
        <button v-if="step > 1" type="button" class="c-btn" @click="step--">{{ $t('org.apply.prev') }}</button>
        <button v-if="step < 4" type="button" class="c-btn" :disabled="busy || (step < 3 && !stepOk)" @click="step === 1 ? next() : step++">{{ $t('org.apply.next') }}</button>
      </div>
    </template>

    <section v-else class="c-card" role="status">
      <h2 class="c-h2">{{ org.display_name }}</h2>
      <template v-if="status === 'pending'">
        <p>{{ $t('org.apply.pending') }}</p>
        <button type="button" class="c-btn" :disabled="busy" @click="withdraw">{{ $t('org.apply.withdraw') }}</button>
      </template>
      <template v-else-if="status === 'verified'">
        <p>{{ $t('org.apply.verified') }}</p>
        <button type="button" class="c-btn" @click="switchTo(org.id)">{{ $t('org.apply.switch') }}</button>
      </template>
      <template v-else>
        <p>{{ $t('org.apply.rejected') }}</p>
        <p v-if="org.review_note">{{ org.review_note }}</p>
        <button v-if="canReapply" type="button" class="c-btn" :disabled="busy" @click="reapply">{{ $t('org.apply.reapply') }}</button>
        <p v-else class="c-mute">{{ $t('org.apply.resubmit', { date: org.resubmit_after.slice(0, 10) }) }}</p>
      </template>
    </section>
  </main>
</template>
