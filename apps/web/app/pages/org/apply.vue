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
const permitKey = ref(0) // 新增許可後換 key，清掉檔案欄上殘留的檔名
const regName = ref('') // 立案證書上傳中顯示檔名；上傳完成即清空（檔案改列在清單上）
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
  regName.value = file?.name ?? ''
  run(async () => { await upload(kind, file); await reload() }).finally(() => { input.value = ''; regName.value = '' })
}
const delDoc = (id: string) => run(async () => { await api(`/orgs/${org.value.id}/documents/${id}`, { method: 'DELETE' }); await reload() })
const addPermit = () => run(async () => {
  const d = await upload('permit', permitFile.value ?? undefined)
  if (!d) return
  try { await api(`/orgs/${org.value.id}/permits`, { method: 'POST', body: { ...permit, document_id: d.id } }) } catch (e) {
    await api(`/orgs/${org.value.id}/documents/${d.id}`, { method: 'DELETE' }).catch(() => {}) // 不留孤兒文件
    throw e
  }
  Object.assign(permit, { permit_no: '', valid_from: '', valid_to: '' }); permitFile.value = null; permitKey.value++
  await reload()
})
const canReapply = computed(() => !org.value?.resubmit_after || Date.now() >= Date.parse(org.value.resubmit_after))
const reapply = () => run(async () => { await api(`/orgs/${org.value.id}`, { method: 'PATCH', body: { legal_name: org.value.legal_name } }); await reload(true) })
const submit = () => run(async () => { await api(`/orgs/${org.value.id}/submit`, { method: 'POST' }); await reload(); await fetchMe(true) })
const withdraw = () => run(async () => { await api(`/orgs/${org.value.id}/withdraw`, { method: 'POST' }); await reload(); await fetchMe(true) })
</script>

<template>
  <main class="c-page ap">
    <CreatorHeader :title="$t('org.apply.title')" back="/org" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="loading" class="c-mute">{{ $t('org.apply.loading') }}</p>

    <template v-else-if="wizard">
      <p v-if="status === 'needs_changes'" class="c-card" role="status"><strong>{{ $t('org.apply.needsChanges') }}</strong><br>{{ org.review_note }}</p>
      <div class="ap-steps" aria-hidden="true"><i v-for="n in 4" :key="n" :class="{ on: n <= step, cur: n === step }"><b>{{ String(n).padStart(2, '0') }}</b></i></div>
      <p class="c-mute ap-stepof">{{ $t('org.apply.stepOf', { n: step }) }}<span v-for="n in 4" :key="n" :class="{ cur: n === step }" :aria-current="n === step ? 'step' : undefined">{{ $t(`org.apply.s${n}`) }}</span></p>

      <section v-for="(fs, i) in STEPS" v-show="step === i + 1" :key="i" class="c-card ap-sec">
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

      <section v-show="step === 3" class="c-card ap-s3">
        <h2 class="c-h2">{{ $t('org.apply.s3') }}</h2>
        <p class="c-mute">{{ $t('org.apply.fileHint') }}</p>
        <h3 id="ap-reg">{{ $t('org.apply.regDoc') }}</h3>
        <ul><li v-for="d in docs('registration')" :key="d.id">{{ d.content_type }} · {{ Math.ceil(d.size_bytes / 1024) }} KB
          <button type="button" class="c-btn" @click="delDoc(d.id)">{{ $t('org.apply.remove') }}</button></li></ul>
        <label class="ap-file" :class="{ off: busy }">
          <input type="file" accept="application/pdf,image/jpeg,image/png" aria-labelledby="ap-reg" :disabled="busy" @change="onFile('registration', $event)">
          <span class="ap-file-btn" aria-hidden="true">{{ $t('org.apply.chooseFile') }}</span><span class="ap-file-name" aria-hidden="true">{{ regName || $t('org.apply.noFile') }}</span>
        </label>
        <h3 id="ap-permit">{{ $t('org.apply.permitDoc') }}</h3>
        <ul><li v-for="p in permits" :key="p.id">{{ p.permit_no }}　{{ p.valid_from }} ~ {{ p.valid_to }}</li></ul>
        <div class="ap-permit">
          <label class="c-field"><span>{{ $t('org.apply.permitNo') }}</span><input v-model="permit.permit_no"></label>
          <label class="c-field"><span>{{ $t('org.apply.validFrom') }}</span><input v-model="permit.valid_from" type="date"></label>
          <label class="c-field"><span>{{ $t('org.apply.validTo') }}</span><input v-model="permit.valid_to" type="date"></label>
        </div>
        <label :key="permitKey" class="ap-file" :class="{ off: busy }">
          <input type="file" accept="application/pdf,image/jpeg,image/png" aria-labelledby="ap-permit" :disabled="busy" @change="onFile('permit', $event)">
          <span class="ap-file-btn" aria-hidden="true">{{ $t('org.apply.chooseFile') }}</span><span class="ap-file-name" :class="{ on: permitFile }" aria-hidden="true">{{ permitFile?.name || $t('org.apply.noFile') }}</span>
        </label>
        <button type="button" class="c-btn" :disabled="busy || !permit.permit_no || !permit.valid_from || !permit.valid_to || !permitFile" @click="addPermit">{{ $t('org.apply.addPermit') }}</button>
      </section>

      <section v-show="step === 4" class="c-card ap-s4">
        <h2 class="c-h2">{{ $t('org.apply.s4') }}</h2>
        <dl class="c-dl"><template v-for="f in FIELDS" :key="f"><template v-if="form[f]"><dt class="c-mute">{{ $t(`org.apply.f.${f}`) }}</dt><dd>{{ f === 'org_type' ? $t(`org.apply.type.${form[f]}`) : form[f] }}</dd></template></template>
          <!-- 送出前確認文件也到齊（第 3 步的上傳結果） -->
          <dt class="c-mute">{{ $t('org.apply.regDoc') }}</dt><dd><span v-for="d in docs('registration')" :key="d.id">{{ d.content_type }} · {{ Math.ceil(d.size_bytes / 1024) }} KB</span><span v-if="!docs('registration').length">—</span></dd>
          <dt class="c-mute">{{ $t('org.apply.permitDoc') }}</dt><dd><span v-for="pm in permits" :key="pm.id">{{ pm.permit_no }}　{{ pm.valid_from }} ~ {{ pm.valid_to }}</span><span v-if="!permits.length">—</span></dd>
        </dl>
        <label class="c-switch"><input v-model="agree" type="checkbox"> {{ $t('org.apply.agree') }}</label>
        <p class="c-mute">{{ $t('org.apply.reviewEta') }}</p>
        <button type="button" class="c-btn primary" :disabled="busy || !agree" @click="submit">{{ $t('org.apply.submit') }}</button>
      </section>

      <div class="c-row ap-nav">
        <button v-if="step > 1" type="button" class="c-btn" @click="step--">{{ $t('org.apply.prev') }}</button>
        <button v-if="step < 4" type="button" class="c-btn primary" :disabled="busy || (step < 3 && !stepOk)" @click="step === 1 ? next() : step++">{{ $t('org.apply.next') }}</button>
      </div>
    </template>

    <section v-else class="c-card ap-state" role="status">
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

<style scoped>
.ap-steps{display:grid;grid-template-columns:repeat(4,1fr);gap:6px;margin:0 0 10px}
.ap-steps i{display:block;height:8px;background:var(--hatch),var(--bg-alt);position:relative}
.ap-steps i.on{background:var(--stripe)}
.ap-steps i b{position:absolute;top:14px;left:0;font:600 11px var(--mono);letter-spacing:.1em;color:var(--mute)}
.ap-steps i.cur b{color:var(--red-text)}
.ap-stepof{margin:34px 0 18px;font:500 12px/1.6 var(--mono);letter-spacing:.06em}
.ap-stepof span{white-space:nowrap}
.ap-stepof span::before{content:"/" / "";margin:0 8px;color:var(--line)}
.ap-stepof span:first-of-type::before{content:"";margin:0 0 0 12px}
.ap-stepof span.cur{color:var(--fg);font-weight:700}
.ap-sec,.ap-s3,.ap-s4{padding:24px clamp(18px,3vw,32px) 12px;border-top:2px solid var(--ink-line)}
.ap-sec{display:grid;grid-template-columns:1fr;gap:0 24px}
.ap-sec>h2,.ap-s3>h2,.ap-s4>h2{grid-column:1/-1;font-size:22px;margin-bottom:22px}
@media (min-width:768px){.ap-sec{grid-template-columns:1fr 1fr}.ap-sec>label:has(textarea){grid-column:1/-1}}
.ap-sec textarea{resize:vertical;min-height:112px}
.ap-s3 h3{font:600 11px var(--mono);letter-spacing:.14em;text-transform:uppercase;color:var(--mute);margin:22px 0 8px;padding-bottom:6px;border-bottom:1px solid var(--line)}
.ap-s3 ul{list-style:none;padding:0;margin:0 0 12px}
.ap-s3 li{display:flex;align-items:center;justify-content:space-between;gap:12px;min-height:48px;border-bottom:1px dotted var(--line);font:500 13px var(--mono)}
.ap-file{position:relative;display:flex;align-items:center;gap:14px;width:100%;box-sizing:border-box;padding:12px 16px;margin:0 0 16px;border:1px dashed var(--ink-line);background:var(--hatch),var(--bg-alt);color:var(--mute);font:500 12px var(--mono);cursor:pointer;min-height:64px}
.ap-file input{position:absolute;inset:0;width:100%;height:100%;opacity:0;cursor:pointer}
.ap-file:focus-within{outline:2px solid var(--red);outline-offset:2px}
.ap-file.off{opacity:.5;cursor:progress}
.ap-file-btn{flex:none;display:inline-flex;align-items:center;min-height:40px;padding:0 16px;border:1px solid var(--ink-line);background:var(--card);color:var(--fg);font:700 13px var(--font);letter-spacing:.06em}
.ap-file-name{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.ap-file-name.on{color:var(--fg)}
@media (hover:hover) and (pointer:fine){.ap-file:not(.off):hover .ap-file-btn{background:var(--fg);color:var(--bg)}}
.ap-s3>.c-btn{margin-bottom:12px}
@media (min-width:768px){.ap-permit{display:grid;grid-template-columns:2fr 1fr 1fr;gap:0 16px}}
.ap-s4 dl.c-dl{margin:0 0 20px}
.ap-s4 dd{font-family:var(--mono);font-size:13px;overflow-wrap:anywhere;margin:0;border-bottom:1px dotted var(--line);padding-bottom:6px}
.ap-s4 dt{border-bottom:1px dotted var(--line);padding-bottom:6px}
.ap-s4 dd span{display:block}
/* 手機：標籤在上、值在下，避免窄欄把網址、email 斷成一字一行 */
@media (max-width:767px){.ap-s4 dl.c-dl{grid-template-columns:1fr;gap:0}.ap-s4 dt{border-bottom:0;padding:10px 0 2px}.ap-s4 dd{padding-bottom:10px}}
.ap-nav{justify-content:space-between;margin-top:8px}
.ap-nav .c-btn{min-width:120px}
.ap-nav .c-btn.primary:only-child,.ap-nav .c-btn.primary:last-child{margin-left:auto}
.ap-state{padding:28px clamp(18px,3vw,32px);border-top:2px solid var(--ink-line)}
.ap-state>h2{font-size:clamp(24px,4vw,34px);letter-spacing:-.02em}
.ap-state>p{font-size:17px;max-width:34em}
.ap-state .c-btn{margin-top:8px}
</style>
