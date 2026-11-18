<script setup lang="ts">
import '~/assets/console.css' // 後台台帳／KPI 樣式（.c-admin、.k-）
// 客服工單（#92）：所有營運角色可讀；回覆並結案限 support（admin 一律可）。
// 會員工單結案會發站內通知；未登入工單系統不寄信，營運先用客服信箱回信，這裡只記處理內容
definePageMeta({ middleware: 'staff' })
const { t } = useI18n()
useHead({ title: () => t('admin.support.title') })
const { api } = useApi()
const { user: me } = useAuth()
const canReply = computed(() => hasStaffRole(me.value, 'support'))
// 脈絡連結只給進得了目標頁的角色（檢舉頁 moderator、金流頁 finance），避免點進去被擋
const canMod = computed(() => hasStaffRole(me.value, 'moderator'))
const canFinance = computed(() => hasStaffRole(me.value, 'finance'))
// 檢舉頁的「對象」檢視（附解除停權 / 恢復清單）；已處置的看 actioned，否則看 open
const reportsOf = (kind: 'user' | 'wishlist', id: string, label: string, acted: boolean) =>
  ({ path: '/admin/reports', query: { status: acted ? 'actioned' : 'open', view: 'rows', target_kind: kind, target_id: id, target_label: label } })
const topupsOf = (uid: string) => ({ path: '/admin/finance', query: { tab: 'transactions', user_id: uid } })
// 預設待回覆（營運先看要處理的）；?status=all|closed 可直接開其他檢視
const qs = useRoute().query.status
const status = ref<'all' | 'open' | 'closed'>(qs === 'all' || qs === 'closed' ? qs : 'open')
const rows = ref<any[]>([]), cursor = ref<string | null>(null), loading = ref(false)
const err = ref(''), msg = ref('')

async function load(more = false) {
  loading.value = true; err.value = ''
  try {
    const r = await api('/admin/support-requests', { query: { status: status.value, limit: 20, cursor: more ? cursor.value : undefined } })
    rows.value = more ? [...rows.value, ...r.data] : r.data; cursor.value = r.next_cursor ?? null
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
watch(status, () => load())
onMounted(() => load())

const mailto = (r: any) => `mailto:${r.contact_email}?subject=${encodeURIComponent(t('common.supportSubject'))}`
const act = ref<any>(null), reply = ref(''), busy = ref(false), aerr = ref('')
function open(r: any) { act.value = r; reply.value = ''; aerr.value = '' }
async function submit() {
  const row = act.value
  if (!row || !reply.value.trim()) return
  busy.value = true; aerr.value = ''
  try {
    const r = await api(`/admin/support-requests/${row.id}`, { method: 'PATCH', body: { reply: reply.value.trim() } })
    Object.assign(row, r)
    msg.value = t('admin.support.done'); act.value = null
  } catch (e: any) {
    if (e.code === 'INVALID_STATE_TRANSITION') { act.value = null; await load(); err.value = t('admin.support.stale') }
    else aerr.value = errMsg(e)
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.support.title')" back="/admin" />
    <label class="c-field c-narrow"><span>{{ $t('admin.support.statusLabel') }}</span>
      <select v-model="status" :disabled="loading" data-testid="status">
        <option v-for="s in ['all', 'open', 'closed']" :key="s" :value="s">{{ $t(`admin.support.status.${s}`) }}</option>
      </select>
    </label>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">{{ $t('admin.support.retry') }}</button></p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <p v-if="loading && !rows.length" class="c-mute" role="status">{{ $t('admin.support.loading') }}</p>
    <p v-else-if="!rows.length && !err" class="c-center c-mute">{{ $t('admin.support.empty') }}</p>
    <article v-for="r in rows" :key="r.id" class="c-card" :data-status="r.status" :data-kind="r.kind">
      <div class="c-row wrap">
        <strong class="c-grow">{{ r.user?.display_name ?? r.contact_email }}</strong>
        <span class="c-badge">{{ $t(`admin.support.kind.${r.kind}`) }}</span>
        <span class="c-badge">{{ $t(`me.support.categories.${r.category}`) }}</span>
        <span v-if="status === 'all'" class="c-badge" data-testid="row-status">{{ $t(`admin.support.status.${r.status}`) }}</span>
      </div>
      <template v-if="r.user">
        <div class="c-mute admin-id">{{ $t('admin.support.userId') }}：{{ r.user.id }}<span v-if="r.user.handle">・@{{ r.user.handle }}</span></div>
        <div class="c-row wrap c-mt">
          <span v-if="r.user.blocked" class="c-badge hidden" data-testid="blocked">{{ $t('admin.support.blocked') }}</span>
          <span v-if="r.user.wallet?.status === 'frozen'" class="c-badge hidden" data-testid="frozen">{{ $t('admin.support.walletFrozen') }}</span>
          <NuxtLink v-if="canMod" class="c-btn small" data-testid="user-reports" :to="reportsOf('user', r.user.id, r.user.display_name, r.user.blocked)">{{ $t('admin.support.userReports') }}</NuxtLink>
          <NuxtLink v-if="canFinance" class="c-btn small" data-testid="user-topups" :to="topupsOf(r.user.id)">{{ $t('admin.support.userTopups') }}</NuxtLink>
        </div>
      </template>
      <a v-else :href="mailto(r)" class="c-btn small c-mt" data-testid="mailto"><UiIcon name="mail" />{{ $t('admin.support.mailto') }}</a>
      <p class="sp-text">{{ r.body }}</p>
      <dl class="c-dl">
        <dt>{{ $t('admin.support.createdAt') }}</dt><dd>{{ fmtTime(r.created_at) }}</dd>
        <template v-if="r.wishlist">
          <dt>{{ $t('admin.support.wishlist') }}</dt>
          <dd data-testid="wishlist">{{ r.wishlist.title }} <span class="admin-id">/s/{{ r.wishlist.slug }}</span>
            <span v-if="r.wishlist.moderation_status === 'hidden'" class="c-badge hidden" data-testid="wl-hidden">{{ $t('admin.support.wishlistHidden') }}</span>
            <NuxtLink v-if="canMod" data-testid="wl-reports" :to="reportsOf('wishlist', r.wishlist.id, r.wishlist.title, r.wishlist.moderation_status === 'hidden')">{{ $t('admin.support.wishlistReports') }}</NuxtLink>
          </dd>
        </template>
        <template v-else-if="r.wishlist_id"><dt>{{ $t('admin.support.wishlist') }}</dt><dd class="admin-id">{{ r.wishlist_id }}</dd></template>
        <template v-if="r.topup">
          <dt>{{ $t('admin.support.topup') }}</dt>
          <dd data-testid="topup"><span class="admin-id">{{ r.topup.merchant_trade_no }}</span> <span class="c-badge" :class="r.topup.status">{{ $t(`admin.topups.status.${r.topup.status}`, r.topup.status) }}</span>
            <NuxtLink v-if="canFinance" data-testid="topup-link" :to="topupsOf(r.topup.user_id)">{{ $t('admin.support.topupLink') }}</NuxtLink>
          </dd>
        </template>
        <template v-else-if="r.topup_id"><dt>{{ $t('admin.support.topup') }}</dt><dd class="admin-id">{{ r.topup_id }}</dd></template>
        <template v-if="r.handled_at"><dt>{{ $t('admin.support.handledAt') }}</dt><dd>{{ fmtTime(r.handled_at) }}</dd></template>
        <template v-if="r.handled_by"><dt>{{ $t('admin.support.handledBy') }}</dt><dd>{{ r.handled_by.display_name }}</dd></template>
        <template v-if="r.reply"><dt>{{ $t('admin.support.reply') }}</dt><dd class="sp-text">{{ r.reply }}</dd></template>
      </dl>
      <div v-if="r.status === 'open' && canReply" class="c-row c-mt">
        <button class="c-btn primary c-grow" @click="open(r)">{{ $t('admin.support.action') }}</button>
      </div>
    </article>
    <button v-if="cursor" class="c-btn block" :disabled="loading" @click="load(true)">{{ $t('admin.support.loadMore') }}</button>

    <CreatorConfirm :open="!!act" :title="$t('admin.support.action')" :text="act?.kind === 'guest' ? $t('admin.support.confirmGuest', { email: act?.contact_email }) : $t('admin.support.confirmMember')" :ok="$t('admin.support.action')" :busy="busy" :disabled="!reply.trim()" :error="aerr" @close="!busy && (act = null)" @ok="submit">
      <label class="c-field"><span>{{ $t('admin.support.replyLabel') }}</span><textarea v-model="reply" rows="5" maxlength="2000" :disabled="busy" /></label>
    </CreatorConfirm>
  </main>
</template>
<style scoped>
.sp-text{white-space:pre-wrap;overflow-wrap:anywhere}
.admin-id{overflow-wrap:anywhere;font:500 12px var(--mono)}
</style>
