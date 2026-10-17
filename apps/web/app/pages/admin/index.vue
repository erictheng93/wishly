<script setup lang="ts">
definePageMeta({ middleware: 'staff' })
useHead({ title: '營運後台' })
const { api } = useApi()
const { user: me } = useAuth()
const tab = ref<'reports' | 'lists' | 'users' | 'flags'>('reports')
const err = ref(''), msg = ref('')
const mask = (e: string) => e.replace(/^(.).*(@.*)$/, '$1***$2')

// 檢舉佇列
const rStatus = ref('open')
const reports = ref<any[]>([]), rCursor = ref<string | null>(null), rLoading = ref(false)
const stats = ref<any>(null), statsErr = ref(''), statsBusy = ref(false)
const STAT: Record<string, string> = { users: '使用者', wishlists: '清單總數', active_wishlists: '使用中清單', hidden_wishlists: '已下架清單', open_reports: '待處理檢舉' }
async function loadStats() {
  statsBusy.value = true; statsErr.value = ''
  try { stats.value = await api('/admin/stats') }
  catch (e) { statsErr.value = errMsg(e) } finally { statsBusy.value = false }
}
const REASON: Record<string, string> = { scam: '疑似詐騙', inappropriate: '不當內容', copyright: '侵害著作權', personal_info: '洩漏個人資料', other: '其他' }
async function loadReports(more = false) {
  rLoading.value = true; err.value = ''
  try {
    const r = await api('/admin/reports', { query: { status: rStatus.value, cursor: more ? rCursor.value : undefined } })
    reports.value = more ? [...reports.value, ...r.data] : r.data; rCursor.value = r.next_cursor
  } catch (e) { err.value = errMsg(e) } finally { rLoading.value = false }
}
async function handle(r: any, status: 'actioned' | 'dismissed') {
  err.value = ''
  try { await api(`/admin/reports/${r.id}`, { method: 'PATCH', body: { status } }); await Promise.all([loadReports(), loadStats()]) }
  catch (e) { err.value = errMsg(e) }
}

// 下架 / 恢復
const mod = ref<{ id: string; title: string; hidden: boolean } | null>(null)
const reason = ref(''), modBusy = ref(false), modErr = ref('')
function openMod(w: any) { mod.value = { id: w.id, title: w.title, hidden: w.moderation_status === 'hidden' }; reason.value = ''; modErr.value = '' }
async function doMod() {
  modBusy.value = true; modErr.value = ''
  try {
    const hide = !mod.value!.hidden
    await api(`/admin/wishlists/${mod.value!.id}/moderation`, { method: 'PATCH', body: hide ? { moderation_status: 'hidden', reason: reason.value.trim() } : { moderation_status: 'ok' } })
    mod.value = null; msg.value = hide ? '已下架' : '已恢復上架'
    await Promise.all([loadReports(), loadStats(), tab.value === 'lists' ? searchLists() : 0])
  } catch (e) { modErr.value = errMsg(e) } finally { modBusy.value = false }
}

// 清單搜尋與分頁
const lq = ref(''), lists = ref<any[]>([]), lDone = ref(false), lCursor = ref<string | null>(null), lBusy = ref(false)
const lStatus = ref(''), lMod = ref('')
async function searchLists(more = false) {
  if (lBusy.value) return
  lBusy.value = true; err.value = ''
  try {
    const r = await api('/admin/wishlists', { query: { q: lq.value.trim() || undefined, status: lStatus.value || undefined, moderation_status: lMod.value || undefined, cursor: more ? lCursor.value : undefined } })
    lists.value = more ? [...lists.value, ...r.data] : r.data; lCursor.value = r.next_cursor; lDone.value = true
  } catch (e) { err.value = errMsg(e) } finally { lBusy.value = false }
}
watch([lq, lStatus, lMod], () => { lCursor.value = null })
// 使用者查詢與操作
const uq = ref(''), users = ref<any[]>([]), shown = ref<Record<string, boolean>>({}), uDone = ref(false), uCursor = ref<string | null>(null), uBusy = ref(false)
const uStatus = ref(''), uProvider = ref('')
const PROVIDER: Record<string, string> = { email: 'Email', line: 'LINE', google: 'Google' }
async function searchUsers(more = false) {
  if (uBusy.value) return
  uBusy.value = true; err.value = ''
  try {
    const r = await api('/admin/users', { query: { q: uq.value.trim() || undefined, status: uStatus.value || undefined, provider: uProvider.value || undefined, cursor: more ? uCursor.value : undefined } })
    users.value = more ? [...users.value, ...r.data] : r.data; uCursor.value = r.next_cursor; uDone.value = true; shown.value = {}
  } catch (e) { err.value = errMsg(e) } finally { uBusy.value = false }
}
watch([uq, uStatus, uProvider], () => { uCursor.value = null })
const userAct = ref<{ user: any; kind: string; title: string; provider?: string } | null>(null)
const userReason = ref(''), userBusy = ref(false), userErr = ref('')
function askUser(user: any, kind: string, title: string, provider?: string) {
  userAct.value = { user, kind, title, provider }; userReason.value = ''; userErr.value = ''
}
async function doUser() {
  if (userBusy.value || !userAct.value) return
  userBusy.value = true; userErr.value = ''
  const { user, kind, provider, title } = userAct.value
  try {
    const path = `/admin/users/${user.id}`
    const body: any = { reason: userReason.value.trim() }
    if (kind === 'blocked') body.blocked = !user.blocked
    if (kind === 'staff') body.is_staff = !user.is_staff
    await api(kind === 'unlink' ? `${path}/identities/${provider}` : kind === 'recovery' ? `${path}/recovery` : kind === 'mfa' ? `${path}/mfa` : path,
      { method: kind === 'unlink' || kind === 'mfa' ? 'DELETE' : kind === 'recovery' ? 'POST' : 'PATCH', body })
    userAct.value = null; msg.value = kind === 'recovery' ? '已申請寄送復原驗證信，請使用者到忘記密碼頁完成重設' : `已完成：${title}`
    await Promise.all([searchUsers(), loadStats()])
  } catch (e) { userErr.value = errMsg(e) } finally { userBusy.value = false }
}
// 系統旗標
const readOnly = ref<boolean | null>(null), flagAsk = ref(false), flagBusy = ref(false)
async function setFlag() {
  flagBusy.value = true; err.value = ''
  try { const r = await api('/admin/system-flags/read_only', { method: 'PUT', body: { value: !readOnly.value } }); readOnly.value = r.value; flagAsk.value = false }
  catch (e) { err.value = errMsg(e) } finally { flagBusy.value = false }
}
async function loadFlags() {
  err.value = ''
  try { readOnly.value = !!(await api('/admin/system-flags')).data.find((f: any) => f.key === 'read_only')?.value } catch (e) { readOnly.value = null; err.value = errMsg(e) }
}
watch(rStatus, () => loadReports())
watch(tab, t => { if (t === 'flags') loadFlags() })
onMounted(() => Promise.all([loadReports(), loadStats()]))
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader title="營運後台" back="/dashboard"><span class="c-badge">STAFF</span></CreatorHeader>
    <p v-if="me && !me.mfa_enabled" class="c-card" role="status">建議啟用兩步驟驗證：後台可停權、發放點數，帳號被盜影響很大。<NuxtLink to="/admin/mfa">現在設定</NuxtLink></p>
    <section class="c-card" aria-label="營運統計">
      <div class="c-row c-mb"><h2 class="c-h2 c-grow c-m0">營運概況</h2><button class="c-btn small" :disabled="statsBusy" @click="loadStats">{{ statsBusy ? '更新中…' : '更新統計' }}</button></div>
      <p v-if="statsErr" class="c-err" role="alert">{{ statsErr }}</p>
      <dl class="admin-stats">
        <div v-for="(name, key) in STAT" :key="key"><dt>{{ name }}</dt><dd :data-testid="`stat-${key}`">{{ stats?.[key] ?? '–' }}</dd></div>
      </dl>
      <details v-if="stats"><summary>捐助、通知與最近 7 天營運動作</summary>
        <div v-for="(name, key) in { claims_by_status: '捐助狀態', notifications_by_status: '通知狀態', audit_last_7d: '最近 7 天稽核動作' }" :key="key" class="c-mt">
          <strong>{{ name }}</strong><ul><li v-for="(count, status) in stats[key]" :key="status">{{ status }}：{{ count }}</li></ul>
          <p v-if="!Object.keys(stats[key] || {}).length" class="c-mute">目前沒有紀錄</p>
        </div>
      </details>
    </section>
    <div class="c-row wrap c-mb">
      <NuxtLink to="/admin/orders" class="c-btn c-grow">採購單佇列</NuxtLink>
      <NuxtLink to="/admin/wallets" class="c-btn c-grow">點數錢包</NuxtLink>
      <NuxtLink to="/admin/audit" class="c-btn c-grow">稽核紀錄</NuxtLink>
    </div>
    <nav class="c-tabs" aria-label="後台功能">
      <button v-for="t in [['reports', '檢舉佇列'], ['lists', '清單搜尋'], ['users', '使用者'], ['flags', '系統旗標']]" :key="t[0]" class="c-btn" :class="{ on: tab === t[0] }" @click="tab = t[0] as any">{{ t[1] }}</button>
    </nav>
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>

    <section v-if="tab === 'reports'">
      <label class="c-field"><span>狀態</span><select v-model="rStatus"><option value="open">待處理</option><option value="actioned">已處理</option><option value="dismissed">已駁回</option></select></label>
      <p v-if="rLoading" class="c-mute">載入中…</p>
      <p v-else-if="!reports.length" class="c-center c-mute">目前沒有{{ rStatus === 'open' ? '待處理的' : '' }}檢舉</p>
      <article v-for="r in reports" :key="r.id" class="c-card">
        <div class="c-row"><strong class="c-grow">{{ REASON[r.reason] || r.reason }}</strong><CreatorStatusBadge :v="r.status" /></div>
        <div>清單：<a :href="`/s/${r.wishlist.slug}`" target="_blank" rel="noopener">{{ r.wishlist.title }}</a>（{{ r.wishlist.slug }}）</div>
        <div v-if="r.detail" class="c-mute">{{ r.detail }}</div>
        <div class="c-mute">{{ new Date(r.created_at).toLocaleString('zh-TW') }}・檢舉者：{{ r.reporter }}{{ r.item_id ? '・針對品項' : '' }}</div>
        <div v-if="r.status === 'open'" class="c-row wrap c-mt">
          <button class="c-btn danger c-grow" @click="openMod({ id: r.wishlist.id, title: r.wishlist.title, moderation_status: 'ok' })">下架</button>
          <button class="c-btn c-grow" @click="handle(r, 'actioned')">已處理</button>
          <button class="c-btn c-grow" @click="handle(r, 'dismissed')">駁回</button>
        </div>
      </article>
      <button v-if="rCursor" class="c-btn block" @click="loadReports(true)">載入更多</button>
    </section>

    <section v-if="tab === 'lists'">
      <form class="c-row" @submit.prevent="searchLists()"><input v-model="lq" class="c-grow c-input" placeholder="slug 或建立者 Email 前綴" aria-label="搜尋清單" :disabled="lBusy"><button class="c-btn primary" :disabled="lBusy">{{ lBusy ? '搜尋中…' : '搜尋' }}</button></form>
      <div class="admin-filters c-mt">
        <label class="c-field"><span>清單狀態</span><select v-model="lStatus" :disabled="lBusy"><option value="">全部</option><option value="draft">草稿</option><option value="active">使用中</option><option value="closed">已結束</option><option value="archived">已封存</option></select></label>
        <label class="c-field"><span>下架狀態</span><select v-model="lMod" :disabled="lBusy"><option value="">全部</option><option value="ok">正常</option><option value="hidden">已下架</option></select></label>
      </div>
      <p v-if="lDone && !lists.length" class="c-center c-mute">沒有符合的清單</p>
      <article v-for="w in lists" :key="w.id" class="c-card c-mt12">
        <div class="c-row"><strong class="c-grow">{{ w.title }}</strong><CreatorStatusBadge :v="w.moderation_status === 'hidden' ? 'hidden' : w.status" /></div>
        <div class="c-mute">{{ w.slug }}・{{ mask(w.owner.email || '') }}・被檢舉 {{ w.open_report_count }} 次</div>
        <div class="c-row c-mt">
          <a :href="`/s/${w.slug}`" target="_blank" rel="noopener" class="c-btn">公開頁</a>
          <button class="c-btn c-grow" :class="{ danger: w.moderation_status !== 'hidden' }" @click="openMod(w)">{{ w.moderation_status === 'hidden' ? '恢復上架' : '下架' }}</button>
        </div>
      </article>
      <button v-if="lCursor" class="c-btn block" :disabled="lBusy" @click="searchLists(true)">載入更多清單</button>
    </section>

    <section v-if="tab === 'users'">
      <form class="c-row" @submit.prevent="searchUsers()"><input v-model="uq" class="c-grow c-input" placeholder="Email、名稱、handle 或 UUID" aria-label="搜尋使用者" :disabled="uBusy"><button class="c-btn primary" :disabled="uBusy">{{ uBusy ? '搜尋中…' : '搜尋' }}</button></form>
      <div class="admin-filters c-mt">
        <label class="c-field"><span>帳號狀態</span><select v-model="uStatus" :disabled="uBusy"><option value="">全部</option><option value="active">正常</option><option value="blocked">已停權</option><option value="deleted">已刪除</option><option value="staff">有效 staff</option></select></label>
        <label class="c-field"><span>登入來源</span><select v-model="uProvider" :disabled="uBusy"><option value="">全部</option><option v-for="(name, key) in PROVIDER" :key="key" :value="key">{{ name }}</option></select></label>
      </div>
      <p v-if="uDone && !users.length" class="c-center c-mute">沒有符合的使用者</p>
      <article v-for="u in users" :key="u.id" class="c-card c-mt12">
        <strong>{{ u.display_name }}</strong> <span v-if="u.is_staff" class="c-badge">STAFF</span> <span v-if="u.blocked" class="c-badge hidden">已封鎖</span> <span v-if="u.deleted" class="c-badge">已刪除</span>
        <div class="c-mute">{{ shown[u.id] ? u.email : mask(u.email || '') }} <button v-if="u.email" class="c-btn small" @click="shown[u.id] = !shown[u.id]">{{ shown[u.id] ? '隱藏' : '顯示' }}</button></div>
        <div class="c-mute admin-id">{{ u.id }}<span v-if="u.handle">・@{{ u.handle }}</span></div>
        <div class="c-mute">登入方式：{{ u.identities.map((i: any) => PROVIDER[i.provider]).join('、') || '無' }}</div>
        <div class="c-row wrap c-mt c-mb" v-if="!u.deleted">
          <button class="c-btn" :class="{ danger: !u.blocked }" @click="askUser(u, 'blocked', u.blocked ? '解除停權' : '停權')">{{ u.blocked ? '解除停權' : '停權' }}</button>
          <button class="c-btn" @click="askUser(u, 'staff', u.is_staff ? '取消 staff' : '設為 staff')">{{ u.is_staff ? '取消 staff' : '設為 staff' }}</button>
          <button v-for="i in u.identities.filter((i: any) => i.provider !== 'email')" :key="i.provider" class="c-btn" :disabled="u.identities.length <= 1" @click="askUser(u, 'unlink', `解除 ${PROVIDER[i.provider]} 綁定`, i.provider)">解除 {{ PROVIDER[i.provider] }} 綁定</button>
          <button class="c-btn" :disabled="!u.email || u.blocked" @click="askUser(u, 'recovery', '寄送復原驗證信')">寄送復原驗證信</button>
          <button v-if="u.mfa_enabled" class="c-btn" @click="askUser(u, 'mfa', '重設兩步驟驗證')">重設兩步驟驗證</button>
        </div>
        <p v-if="!u.email && !u.deleted" class="c-mute">無已驗證 Email；帳號復原須另行核驗身分。</p>
        <div class="c-mute">清單 {{ u.wishlist_count }}・{{ new Date(u.created_at).toLocaleDateString('zh-TW') }}</div>
      </article>
      <button v-if="uCursor" class="c-btn block" :disabled="uBusy" @click="searchUsers(true)">載入更多使用者</button>
    </section>

    <section v-if="tab === 'flags'" class="c-card">
      <label class="c-switch"><input type="checkbox" :checked="!!readOnly" :disabled="readOnly === null || flagBusy" @click.prevent="flagAsk = true"><span>唯讀模式（read_only）<br><small class="c-mute">開啟後所有非安全方法回 503，全站無法寫入。目前狀態{{ readOnly === null ? '未知（切換後顯示）' : readOnly ? '開啟' : '關閉' }}。</small></span></label>
      <p class="c-mute">所有營運動作都會寫入稽核紀錄。</p>
    </section>

    <CreatorConfirm :open="!!mod" :title="mod?.hidden ? '恢復上架' : '下架清單'" :text="`清單：${mod?.title}`" :ok="mod?.hidden ? '恢復上架' : '下架此清單'" :danger="!mod?.hidden" :busy="modBusy" :disabled="!mod?.hidden && !reason.trim()" :error="modErr" @close="mod = null" @ok="doMod">
      <label v-if="mod && !mod.hidden" class="c-field"><span>下架原因（必填，會顯示給建立者）</span><input v-model="reason" maxlength="200"></label>
    </CreatorConfirm>
    <CreatorConfirm :open="!!userAct" :title="userAct?.title || ''" :text="`對象：${userAct?.user.display_name}。停權、staff 變更、解除綁定與重設兩步驟驗證會撤銷現有登入，使用者須重新登入。`" ok="確認執行" danger :busy="userBusy" :disabled="!userReason.trim()" :error="userErr" @close="!userBusy && (userAct = null)" @ok="doUser">
      <label class="c-field"><span>操作原因</span><input v-model="userReason" maxlength="200" :disabled="userBusy" required></label>
    </CreatorConfirm>
    <CreatorConfirm :open="flagAsk" title="切換唯讀模式" :text="readOnly ? '確定關閉唯讀模式？' : '唯讀模式會影響全站寫入，確定開啟？'" :busy="flagBusy" danger @close="flagAsk = false" @ok="setFlag" />
  </main>
</template>

<style scoped>
.admin-stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(120px,1fr));gap:16px;margin:0 0 16px}
.admin-stats dt{font-size:13px;color:var(--mute)}
.admin-stats dd{margin:4px 0 0;font-size:24px;font-weight:700;font-variant-numeric:tabular-nums}
.admin-filters{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:12px}
.admin-id{overflow-wrap:anywhere}
</style>
