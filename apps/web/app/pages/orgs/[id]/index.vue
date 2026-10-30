<script setup lang="ts">
// 組織後台總覽：事件與據點；owner / coordinator 可新增，volunteer 唯讀
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('org.console.title') })
const { api } = useApi()
const id = useRoute().params.id as string
const { orgs, current, adopt } = useOrgMode()
const role = computed(() => orgs.value.find(o => o.id === id)?.role)
const canEdit = computed(() => role.value === 'owner' || role.value === 'coordinator')
const org = ref<any>(null), sites = ref<any[]>([]), events = ref<any[]>([])
const loading = ref(true), err = ref(''), msg = ref(''), busy = ref(false)
const permits = ref<any[]>([])
const permit = reactive({ permit_no: '', valid_from: '', valid_to: '' })
const permitFile = ref<File | null>(null)
const today = taipeiToday()
const expired = (p: any) => p.valid_to < today
const usable = (p: any) => p.status === 'approved' && p.valid_from <= today && !expired(p)
const hasValid = computed(() => permits.value.some(usable))
// 未關閉、但綁著失效許可的據點：可一鍵換成有效許可
const stale = computed(() => sites.value.filter(s => s.status !== 'closed' && !usable(permits.value.find(p => p.id === s.permit_id) ?? {})))
const loadPermits = async () => { permits.value = await api(`/orgs/${id}/permits`) }
if (current.value !== id) adopt(id)
onMounted(async () => {
  try {
    ;[org.value, sites.value, events.value] = await Promise.all([api(`/orgs/${id}`), api(`/orgs/${id}/sites`), api(`/orgs/${id}/events`)])
    await loadPermits()
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
  if (location.hash === '#permits') nextTick(() => document.getElementById('permits')?.scrollIntoView()) // 區塊載入後才渲染，瀏覽器原生錨點跳不到
})
// 與 org/apply.vue 的 upload / addPermit 相同；已上線組織刪文件會 409，故清理失敗要吞掉
async function upload(file: File) {
  if (file.size > 10 * 1024 * 1024) throw new Error(t('org.apply.fileBig'))
  if (!['application/pdf', 'image/jpeg', 'image/png'].includes(file.type)) throw new Error(t('org.apply.fileType'))
  return await api<any>(`/orgs/${id}/documents`, { method: 'POST', query: { kind: 'permit' }, body: file, headers: { 'Content-Type': file.type } })
}
async function addPermit() {
  err.value = msg.value = ''; busy.value = true
  try {
    const d = await upload(permitFile.value!)
    try { await api(`/orgs/${id}/permits`, { method: 'POST', body: { ...permit, document_id: d.id } }) } catch (e) {
      await api(`/orgs/${id}/documents/${d.id}`, { method: 'DELETE' }).catch(() => {})
      throw e
    }
    Object.assign(permit, { permit_no: '', valid_from: '', valid_to: '' }); permitFile.value = null
    await loadPermits(); msg.value = t('org.console.permitSaved')
  } catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
async function applyPermit(pid: string) {
  err.value = msg.value = ''; busy.value = true
  try {
    const r = await api<any>(`/orgs/${id}/permits/${pid}/apply`, { method: 'POST' })
    sites.value = await api(`/orgs/${id}/sites`); msg.value = t('org.console.permitApplied', { n: r.updated })
  } catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
</script>

<template>
  <main class="c-page oh">
    <CreatorHeader :title="org?.display_name || $t('org.console.title')" />
    <p v-if="err" class="c-err c-errbox" role="alert">{{ err }}</p>
    <p v-if="loading" class="c-mute oe-load" role="status">{{ $t('org.apply.loading') }}</p>
    <template v-else-if="org">
      <nav class="c-row oh-nav">
        <NuxtLink v-if="role === 'owner'" :to="`/orgs/${id}/members`" class="c-btn">{{ $t('org.console.members') }}</NuxtLink>
      </nav>

      <section class="c-card oh-sec">
        <h2 class="c-h2">{{ $t('org.console.events') }}<b class="oh-count" aria-hidden="true">{{ String(events.length).padStart(2, '0') }}</b></h2>
        <NuxtLink v-if="canEdit" :to="`/orgs/${id}/events/new`" class="c-btn primary">{{ $t('org.console.newEvent') }}</NuxtLink>
        <p v-if="!events.length" class="c-mute">{{ $t('org.console.noEvents') }}</p>
        <ul class="c-list oh-list">
          <li v-for="e in events" :key="e.id">
            <strong>{{ e.name }}</strong>
            <span class="c-mute">{{ e.occurred_on }}　{{ (e.counties ?? []).join('、') }}</span>
          </li>
        </ul>
      </section>

      <section class="c-card oh-sec">
        <h2 class="c-h2">{{ $t('org.console.sites') }}<b class="oh-count" aria-hidden="true">{{ String(sites.length).padStart(2, '0') }}</b></h2>
        <NuxtLink v-if="canEdit" :to="`/orgs/${id}/sites/new`" class="c-btn primary">{{ $t('org.console.newSite') }}</NuxtLink>
        <p v-if="!sites.length" class="c-mute">{{ $t('org.console.noSites') }}</p>
        <ul class="c-list oh-list">
          <li v-for="s in sites" :key="s.id">
            <strong>{{ s.title }}</strong>
            <CreatorStatusBadge v-if="s.site_status" :v="s.site_status" />
            <span class="c-mute">{{ s.site_address }}</span>
            <span class="oh-acts">
              <NuxtLink v-if="canEdit" :to="`/lists/${s.id}/edit`">{{ $t('org.console.edit') }}</NuxtLink>
              <NuxtLink :to="`/orgs/${id}/sites/${s.id}/receipts`">{{ $t('org.console.receipts') }}</NuxtLink>
              <NuxtLink :to="`/s/${s.slug}`">{{ $t('org.console.publicPage') }}</NuxtLink>
            </span>
          </li>
        </ul>
      </section>

      <section id="permits" class="c-card oh-sec">
        <h2 class="c-h2">{{ $t('org.console.permits') }}<b class="oh-count" aria-hidden="true">{{ String(permits.length).padStart(2, '0') }}</b></h2>
        <p v-if="!hasValid" class="c-mute">{{ $t('org.console.permitsNone') }}</p>
        <ul class="c-list oh-list">
          <li v-for="p in permits" :key="p.id">
            <strong>{{ p.permit_no }}</strong>
            <span><span :class="['c-badge', p.status]">{{ $t(`org.console.permitSt.${p.status}`) }}</span> <span v-if="p.status === 'approved' && expired(p)" class="c-badge">{{ $t('org.console.permitExpired') }}</span></span>
            <span class="c-mute">{{ p.valid_from }} ~ {{ p.valid_to }}</span>
            <span v-if="p.review_note" class="c-mute">{{ $t('org.console.permitNote', { note: p.review_note }) }}</span>
            <button v-if="canEdit && usable(p) && stale.length" type="button" class="c-btn oh-apply" :disabled="busy" @click="applyPermit(p.id)">{{ $t('org.console.permitApply', { n: stale.length }) }}</button>
          </li>
        </ul>
        <p v-if="msg" class="c-ok oh-msg" role="status">{{ msg }}</p>
        <form v-if="role === 'owner' && org.verification_status === 'verified'" class="oh-form" @submit.prevent="addPermit">
          <label class="c-field"><span>{{ $t('org.apply.permitNo') }}</span><input v-model="permit.permit_no" name="permit_no" required></label>
          <label class="c-field"><span>{{ $t('org.apply.validFrom') }}</span><input v-model="permit.valid_from" name="valid_from" type="date" required></label>
          <label class="c-field"><span>{{ $t('org.apply.validTo') }}</span><input v-model="permit.valid_to" name="valid_to" type="date" required></label>
          <label class="c-field"><span>{{ $t('org.apply.permitDoc') }}</span><span class="oh-file" :class="{ off: busy }"><input type="file" name="permit_file" accept="application/pdf,image/jpeg,image/png" :disabled="busy" @change="permitFile = ($event.target as HTMLInputElement).files?.[0] ?? null"><span class="oh-file-btn" aria-hidden="true">{{ $t('org.apply.chooseFile') }}</span><span class="oh-file-name" :class="{ on: permitFile }" aria-hidden="true">{{ permitFile?.name || $t('org.apply.noFile') }}</span></span></label>
          <button class="c-btn primary" :disabled="busy || !permit.permit_no || !permit.valid_from || !permit.valid_to || !permitFile">{{ $t('org.apply.addPermit') }}</button>
        </form>
      </section>
    </template>
  </main>
</template>

<style scoped>
.oh-form{margin-top:8px;margin-bottom:18px;padding-top:20px;border-top:1px dashed var(--ink-line)}
.oh-file{margin-bottom:0;letter-spacing:0;position:relative;display:flex;align-items:center;gap:14px;box-sizing:border-box;width:100%;min-height:64px;padding:12px 16px;border:1px dashed var(--ink-line);background:var(--hatch),var(--bg-alt);color:var(--mute);font:500 12px var(--mono);cursor:pointer}
.oh-file input{position:absolute;inset:0;width:100%;height:100%;min-height:0;opacity:0;cursor:pointer;padding:0;border:0}
.oh-file:focus-within{outline:2px solid var(--red);outline-offset:2px}
.oh-file.off{opacity:.5;cursor:progress}
.oh-file-btn{flex:none;display:inline-flex;align-items:center;min-height:40px;padding:0 16px;border:1px solid var(--ink-line);background:var(--card);color:var(--fg);font:700 13px var(--font);letter-spacing:.06em}
.oh-file-name{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.oh-file-name.on{color:var(--fg)}
.oh-form>.c-btn{min-height:48px}
.oh-list .c-badge.approved{color:var(--ok);border-color:currentColor}
.oh-list .c-badge.pending{color:var(--red-text);border-color:currentColor}
.oe-load{padding:28px 0;font:600 12px var(--mono);letter-spacing:.14em}

.oh-list li>.oh-apply{grid-column:1/-1;justify-self:start}
.oh-msg{margin-top:12px}
.oh-nav{margin-bottom:16px}
.oh-nav .c-btn{min-height:48px}
.oh-sec{padding:0;border-top:2px solid var(--ink-line);margin-bottom:24px}
.oh-sec>*{margin-left:18px;margin-right:18px}
.oh-sec>.c-h2{margin:0;padding:14px 18px;background:var(--bg-alt);border-bottom:1px solid var(--line)}
.oh-count{margin-left:auto;font:700 13px var(--mono);letter-spacing:.08em;color:var(--mute);border:1px solid var(--line);padding:2px 8px;background:var(--card)}
.oh-sec>.c-btn{margin-top:16px}
.oh-sec>.c-mute{margin-top:16px;margin-bottom:18px}
.oh-list{list-style:none;padding:0;margin:4px 18px 0!important;display:block}
.oh-list li{display:grid;grid-template-columns:auto 1fr;gap:4px 12px;align-items:center;padding:14px 0;border-top:1px solid var(--line);animation:none}
.oh-list li:first-child{border-top:0}
.oh-list li>strong{font-size:17px;grid-column:1}
.oh-list li>.c-mute{grid-column:1/-1;font:500 12px/1.5 var(--mono);letter-spacing:.03em}
.oh-list li>.c-badge{grid-column:2;justify-self:start}
.oh-acts{grid-column:1/-1;display:flex;flex-wrap:wrap;margin-top:2px}
.oh-acts a{display:inline-flex;align-items:center;min-height:44px;padding:0 12px 0 0;margin-right:12px;border-right:1px solid var(--line);font-size:14px;font-weight:700;text-decoration:underline;text-decoration-color:var(--red);text-underline-offset:4px}
.oh-acts a:last-child{border-right:0}
@media (min-width:768px){
  .oh-list li{grid-template-columns:auto auto 1fr}
  .oh-list li>.c-badge{grid-column:2}
  .oh-acts{grid-column:3;grid-row:1/3;justify-self:end;margin:0}
  .oh-list li>.c-mute{grid-column:1/3}
}
</style>
