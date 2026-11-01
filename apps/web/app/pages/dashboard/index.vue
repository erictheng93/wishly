<script setup lang="ts">
import '~/assets/creator-pages.css'
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('lists.dashboard.title') })
const { api } = useApi()
const list = ref<any[]>([])
const cursor = ref<string | null>(null)
const loading = ref(true), err = ref('')
const del = ref<any>(null), delBusy = ref(false), delErr = ref('')
const archived = ref(false) // 切到已封存列表；一般列表由後端排除已封存，分頁才不會被佔位
async function load(more = false) {
  loading.value = true; err.value = ''
  try {
    const r = await api('/wishlists', { query: { limit: 20, cursor: more ? cursor.value : undefined, status: archived.value ? 'archived' : 'draft,active,closed' } })
    list.value = more ? [...list.value, ...r.data] : r.data
    cursor.value = r.next_cursor
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
async function archive() {
  delBusy.value = true; delErr.value = ''
  try { await api(`/wishlists/${del.value.id}`, { method: 'DELETE' }); del.value = null; await load() }
  catch (e) { delErr.value = errMsg(e) } finally { delBusy.value = false }
}
async function unarchive(w: any) {
  err.value = ''
  try { await api(`/wishlists/${w.id}`, { method: 'PATCH', body: { status: 'closed' } }); await load() }
  catch (e) { err.value = errMsg(e) }
}
function toggleArchived() { archived.value = !archived.value; list.value = []; load() }
const pad = (n: number) => String(n).padStart(2, '0')
const pctOf = (w: any) => Math.max(0, Math.min(100, Number(w.completion?.completion_pct ?? 0)))
// 新手導覽：剛註冊、什麼都還沒做時先帶去導覽頁（每個分頁工作階段一次，按返回不會一直被拉回去）
const tour = useOnboarding()
onMounted(async () => {
  load()
  const s = await tour.refresh()
  let welcomed = false
  try { welcomed = !!sessionStorage.getItem('ws_tour_welcomed') } catch {}
  if (tour.active.value && s && s.steps.every(x => !x.done) && !welcomed) navigateTo('/welcome')
})
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="archived ? $t('lists.dashboard.showArchived') : $t('lists.dashboard.title')" />
    <CreatorAccountNudge />
    <div class="dsh-bar">
      <span class="x-idx" aria-hidden="true">INDEX / {{ pad(list.length) }}</span>
      <button class="c-btn small" @click="toggleArchived">{{ archived ? $t('lists.dashboard.backToLists') : $t('lists.dashboard.showArchived') }}</button>
    </div>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">{{ $t('lists.common.retry') }}</button></p>
    <template v-else-if="loading && !list.length">
      <p class="c-mute" role="status">{{ $t('lists.common.loading') }}</p>
      <div class="c-grid dsh-skel" aria-hidden="true"><div v-for="n in 3" :key="n" class="c-card"><i /><i /><i /></div></div>
    </template>
    <div v-else-if="!list.length && archived" class="x-empty"><span class="dsh-zero" aria-hidden="true">00</span><p class="c-m0">{{ $t('lists.dashboard.archivedEmpty') }}</p></div>
    <div v-else-if="!list.length" class="x-empty dsh-empty">
      <span class="dsh-zero" aria-hidden="true">00</span>
      <div class="dsh-ghost" aria-hidden="true"><i /><b /><b /><em /></div>
      <p class="c-m0">{{ $t('lists.dashboard.empty') }}</p>
      <NuxtLink to="/lists/new" class="c-btn primary c-mt12">{{ $t('lists.dashboard.create') }}</NuxtLink>
    </div>
    <div class="c-grid dsh-grid">
    <article v-for="(w, k) in list" :key="w.id" class="c-card link dsh" :class="{ bad: w.moderation_status === 'hidden' }">
      <div class="dsh-top x-tag"><span aria-hidden="true">No.{{ pad(k + 1) }}</span><CreatorStatusBadge :v="w.moderation_status === 'hidden' ? 'hidden' : w.status" /></div>
      <div class="dsh-main">
        <img class="c-thumb dsh-cover" :src="coverSrc(w)" alt="">
        <div class="c-grow">
          <NuxtLink :to="`/lists/${w.id}/edit`" class="c-stretch dsh-title"><strong>{{ w.title }}</strong></NuxtLink>
          <div class="c-mute dsh-kind">{{ $t('lists.common.personal') }}</div>
        </div>
      </div>
      <div class="dsh-gauge">
        <b class="dsh-pct" aria-hidden="true">{{ pctOf(w) }}<small>%</small></b>
        <div class="dsh-gc">
          <div class="c-bar dsh-prog" aria-hidden="true"><i :style="{ width: pctOf(w) + '%' }" /></div>
          <div class="c-mute dsh-meta">{{ $t('lists.dashboard.completion', { pct: w.completion?.completion_pct ?? 0, done: w.completion?.fulfilled_count ?? 0, total: w.completion?.item_count ?? 0 }) }}</div>
        </div>
      </div>
      <div v-if="w.moderation_status === 'hidden'" class="c-err" role="alert">
        {{ $t('lists.dashboard.hidden', { reason: w.moderation_reason ? $t('lists.common.reasonSuffix', { reason: w.moderation_reason }) : '' }) }}
        <SupportLink :topic="w.title" />
      </div>
      <div class="c-row dsh-act">
        <NuxtLink :to="`/lists/${w.id}/progress`" class="c-btn c-grow">{{ $t('lists.common.progress') }}</NuxtLink>
        <button v-if="w.status === 'archived'" class="c-btn" @click="unarchive(w)">{{ $t('lists.dashboard.unarchive') }}</button>
        <button v-else class="c-btn danger" @click="del = w; delErr = ''">{{ $t('lists.dashboard.archive') }}</button>
      </div>
    </article>
    </div>
    <button v-if="cursor" class="c-btn block dsh-more" :disabled="loading" @click="load(true)">{{ $t('lists.dashboard.loadMore') }}</button>
    <NuxtLink v-if="list.length && !archived" to="/lists/new" class="c-btn primary c-fab">{{ $t('lists.dashboard.newFab') }}</NuxtLink>
    <CreatorConfirm :open="!!del" :title="$t('lists.dashboard.archiveTitle')" :text="$t('lists.dashboard.archiveText', { title: del?.title })" :ok="$t('lists.dashboard.archiveOk')" danger :busy="delBusy" :error="delErr" @close="del = null" @ok="archive" />
  </main>
</template>
<style scoped>
.dsh-bar{display:flex;align-items:center;justify-content:space-between;gap:12px;margin:0 0 16px}
.dsh-bar .c-btn{border-color:var(--line);font-size:13px}
.dsh-grid{margin-bottom:16px}
/* 標本檔案卡：編號條 → 封面＋標題 → 大完成率＋斜紋進度 → 動作 */
.dsh{padding:0;display:flex;flex-direction:column;overflow:hidden}
.dsh-top{padding:10px 14px;border-bottom:1px solid var(--ink-line);background:var(--bg-alt)}
.dsh-main{display:flex;gap:14px;align-items:flex-start;padding:14px 14px 0}
.dsh .dsh-cover{width:84px;height:84px;background:var(--bg-alt);outline:1px solid var(--line);outline-offset:-1px;object-fit:cover}
.dsh .dsh-cover[src$='.svg']{object-fit:contain}
.dsh-title{align-items:flex-start!important}
.dsh-title strong{font-size:18px;line-height:1.3;font-weight:800;letter-spacing:-.005em;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}
.dsh-kind{margin-top:2px;font:600 11px/1.4 var(--mono);letter-spacing:.1em;text-transform:uppercase}
.dsh-gauge{display:flex;align-items:flex-end;gap:14px;margin:16px 14px 0;padding-top:14px;border-top:1px dashed var(--line)}
.dsh-pct{flex:none;font-size:54px;line-height:.85;font-weight:900;letter-spacing:-.05em;font-variant-numeric:tabular-nums;color:var(--fg)}
.dsh-pct small{font-size:20px;letter-spacing:0;margin-left:2px;color:var(--red-text)}
.dsh-gc{flex:1;min-width:0;padding-bottom:2px}
.dsh .dsh-prog{height:10px;margin:0 0 8px}
.dsh-meta{font:500 11px/1.4 var(--mono);letter-spacing:.04em}
.dsh .c-err{margin:12px 14px 0}
.dsh-act{margin:16px 14px 14px;margin-top:auto;padding-top:16px}
.dsh.bad .dsh-top{background:var(--bad-bg)}
.dsh-more{margin:0 0 16px}
/* 空狀態：虛線斜線框、巨大 00、幽靈卡片 */
.dsh-empty{padding:44px 20px 40px;margin-top:4px}
.dsh-zero{font:900 clamp(96px,26vw,168px)/.8 var(--font);letter-spacing:-.06em;color:transparent;-webkit-text-stroke:2px var(--ink-line);font-variant-numeric:tabular-nums}
.dsh-ghost{display:grid;grid-template-columns:44px 1fr;grid-template-rows:12px 12px 8px;gap:8px 12px;width:min(260px,70%);margin:8px 0 12px;padding:12px;border:1px solid var(--line);background:var(--card);opacity:.9}
.dsh-ghost i{grid-row:1/3;background:var(--hatch),var(--bg-alt);outline:1px solid var(--line);outline-offset:-1px}
.dsh-ghost b{background:var(--line)}.dsh-ghost b+b{width:60%}
.dsh-ghost em{grid-column:1/3;background:var(--hatch),var(--bg-alt)}
.dsh-skel .c-card{min-height:210px;display:grid;align-content:start;gap:12px}
.dsh-skel i{display:block;height:14px;background:var(--hatch),var(--bg-alt)}
.dsh-skel i:first-child{height:40px}
@media (max-width:767px){.dsh .dsh-cover{width:72px;height:72px}.dsh-pct{font-size:48px}}
</style>
