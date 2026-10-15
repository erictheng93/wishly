<script setup lang="ts">
definePageMeta({ middleware: 'auth' })
useHead({ title: '帳號設定' })
const { api } = useApi()
const { user, fetchMe, logout, oauth } = useAuth()
const route = useRoute()
const providers = [{ provider: 'email', label: 'Email' }, { provider: 'google', label: 'Google' }, { provider: 'line', label: 'LINE' }] as const
const linked = computed(() => new Set<string>((user.value?.identities ?? []).map((i: any) => i.provider)))
const identityBusy = ref(false), identityErr = ref(''), identityMsg = ref('')
const linkEmail = ref(''), sentEmail = ref(''), linkCode = ref(''), cooldown = ref(0)
const linkOtp = ref<{ focus: () => void }>()
const removingProvider = ref('')
const removingLabel = computed(() => providers.find(p => p.provider === removingProvider.value)?.label ?? '')
let linkTimer: ReturnType<typeof setInterval> | undefined
onBeforeUnmount(() => clearInterval(linkTimer))
const name = ref(''), msg = ref(''), err = ref('')
const handle = ref(''), dcv = ref('friends'), hMsg = ref(''), hErr = ref(''), copied = ref(false)
const profileUrl = computed(() => user.value?.handle ? `${siteOrigin()}/u/${user.value.handle}` : '')
const hBad = computed(() => !!handle.value && !isValidHandle(handle.value))
const exporting = ref(false)
const showDel = ref(false), word = ref(''), delBusy = ref(false), delErr = ref('')
const LINK_ERRORS: Record<string, string> = { oauth_failed: '綁定失敗，請重新操作並確認登入帳號', oauth_unavailable: '此登入方式尚未開放', identity_in_use: '此登入方式或 Email 已屬於另一個帳號，無法直接綁定。請使用原帳號登入。', provider_already_linked: '已綁定此登入方式，請先解除後再更換' }
// 網頁版整頁導向供應商再回來；原生殼等 deep link 回來後就地更新
async function linkOAuth(p: (typeof providers)[number]) {
  if (identityBusy.value || p.provider === 'email') return
  identityBusy.value = true; identityErr.value = identityMsg.value = ''
  try {
    await oauth(p.provider, '/settings', true)
    if (isNative()) identityMsg.value = `已綁定 ${p.label}`
  } catch (e: any) {
    if (e.message !== 'cancelled') identityErr.value = LINK_ERRORS[e.message] || errMsg(e)
  } finally { identityBusy.value = false }
}
onMounted(async () => { await fetchMe(true); name.value = user.value?.display_name ?? ''; handle.value = user.value?.handle ?? ''; dcv.value = user.value?.default_claim_visibility ?? 'friends'; linkEmail.value = user.value?.email ?? ''
  identityErr.value = LINK_ERRORS[String(route.query.error)] ?? ''
  const label = providers.find(p => p.provider === route.query.linked)?.label
  if (label) identityMsg.value = `已綁定 ${label}`
})

async function sendLinkEmail() {
  if (identityBusy.value || cooldown.value > 0) return
  identityBusy.value = true; identityErr.value = identityMsg.value = ''
  try {
    const email = sentEmail.value || linkEmail.value.trim()
    const r = await api('/me/identities/email/request', { method: 'POST', body: { email } })
    sentEmail.value = email; linkCode.value = ''; cooldown.value = r.resend_after || 60
    clearInterval(linkTimer)
    linkTimer = setInterval(() => { if (--cooldown.value <= 0) clearInterval(linkTimer) }, 1000)
    nextTick(() => linkOtp.value?.focus())
  } catch (e) { identityErr.value = errMsg(e) } finally { identityBusy.value = false }
}
function changeLinkEmail() { sentEmail.value = ''; linkCode.value = ''; cooldown.value = 0; clearInterval(linkTimer) }
async function verifyLinkEmail() {
  if (identityBusy.value || linkCode.value.length !== 6) return
  identityBusy.value = true; identityErr.value = identityMsg.value = ''
  try {
    user.value = await api('/me/identities/email/verify', { method: 'POST', body: { email: sentEmail.value, code: linkCode.value } })
    sentEmail.value = ''; linkCode.value = ''; cooldown.value = 0; clearInterval(linkTimer)
    identityMsg.value = '已綁定 Email，可收取通知並設定密碼'
  } catch (e) { identityErr.value = errMsg(e); linkCode.value = ''; linkOtp.value?.focus() } finally { identityBusy.value = false }
}
function closeUnlink() { if (!identityBusy.value) removingProvider.value = '' }
async function unlinkIdentity() {
  if (identityBusy.value) return
  identityBusy.value = true; identityErr.value = identityMsg.value = ''
  try {
    user.value = await api(`/me/identities/${removingProvider.value}`, { method: 'DELETE' })
    identityMsg.value = `已解除 ${removingLabel.value} 綁定`
    removingProvider.value = ''; linkEmail.value = user.value.email ?? ''
  } catch (e) { identityErr.value = errMsg(e) } finally { identityBusy.value = false }
}

async function toggle(ev: Event) {
  const v = (ev.target as HTMLInputElement).checked, old = !v
  user.value.notification_prefs = { ...user.value.notification_prefs, email_claims: v } // 樂觀更新
  try { await api('/me', { method: 'PATCH', body: { notification_prefs: { email_claims: v } } }) }
  catch (e) { user.value.notification_prefs = { ...user.value.notification_prefs, email_claims: old }; err.value = errMsg(e) }
}
async function saveName() {
  msg.value = err.value = ''
  try { user.value = await api('/me', { method: 'PATCH', body: { display_name: name.value.trim() } }); msg.value = '已儲存' }
  catch (e) { err.value = errMsg(e) }
}
async function saveSocial() {
  hMsg.value = hErr.value = ''
  if (hBad.value) { hErr.value = '代號只能用小寫英文、數字、底線，3–30 字'; return }
  try {
    user.value = await api('/me', { method: 'PATCH', body: { handle: handle.value || null, default_claim_visibility: dcv.value } })
    hMsg.value = '已儲存'
  } catch (e: any) { hErr.value = apiErrMsg(e) }
}
async function copyUrl() { try { await copyText(profileUrl.value); copied.value = true } catch { hErr.value = '無法複製，請手動選取網址' } }
async function exportData() {
  exporting.value = true; err.value = ''
  try {
    const data = await api('/me/export')
    const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' }))
    const a = Object.assign(document.createElement('a'), { href: url, download: `wishsync-export-${new Date().toISOString().slice(0, 10)}.json` })
    a.click(); URL.revokeObjectURL(url)
  } catch (e) { err.value = errMsg(e) } finally { exporting.value = false }
}
async function remove() {
  delBusy.value = true; delErr.value = ''
  try {
    await api('/me', { method: 'DELETE', body: { confirm: 'DELETE' } })
    user.value = null
    await navigateTo('/')
  } catch (e: any) {
    delErr.value = e.code === 'ACCOUNT_HAS_POINTS' ? `${e.detail}（可到「我的點數」撤回認捐）` : e.code === 'ACCOUNT_HAS_BALANCE' ? `${e.detail}（餘額 ${e.body.balance ?? 0} 點、未完成認捐 ${e.body.pledged_count ?? 0}、未結案訂單 ${e.body.open_order_count ?? 0}）請先處理後再刪除。` : errMsg(e)
  } finally { delBusy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader title="帳號設定" back="/dashboard" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <template v-if="user">
      <section class="c-card">
        <h2 class="c-h2">我的資料</h2>
        <p class="c-mute">{{ user.email || '尚未綁定 Email' }}</p>
        <form @submit.prevent="saveName">
          <label class="c-field"><span>顯示名稱</span><input v-model="name" maxlength="40" required></label>
          <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
          <button class="c-btn">儲存</button>
        </form>
      </section>
      <section class="c-card" aria-labelledby="login-methods-title">
        <h2 id="login-methods-title" class="c-h2">登入方式</h2>
        <p class="c-mute">至少保留一種登入方式。已屬於其他帳號的方式無法直接綁定。</p>
        <ul class="identity-list">
          <li v-for="p in providers" :key="p.provider" class="c-row">
            <span class="c-grow">{{ p.label }} <small class="c-mute">{{ linked.has(p.provider) ? '已綁定' : '未綁定' }}</small></span>
            <button v-if="linked.has(p.provider)" type="button" class="c-btn" :aria-label="`解除 ${p.label} 綁定`" :disabled="identityBusy || linked.size <= 1" @click="removingProvider = p.provider; identityErr = ''">解除綁定</button>
            <button v-else-if="p.provider !== 'email'" type="button" class="c-btn" :disabled="identityBusy" @click="linkOAuth(p)">綁定 {{ p.label }}</button>
          </li>
        </ul>
        <p v-if="linked.size <= 1" class="c-mute">先新增另一種登入方式，才能解除目前的方式。</p>
        <form v-if="!linked.has('email')" @submit.prevent="sentEmail ? verifyLinkEmail() : sendLinkEmail()">
          <label class="c-field"><span>綁定 Email</span><input v-model="linkEmail" type="email" autocomplete="email" maxlength="254" :disabled="!!sentEmail || identityBusy" required></label>
          <template v-if="sentEmail">
            <p class="c-mute">綁定驗證碼已寄到 {{ sentEmail }}</p>
            <CreatorOtpInput ref="linkOtp" v-model="linkCode" @complete="verifyLinkEmail" />
            <button class="c-btn primary block c-mb" :disabled="identityBusy || linkCode.length !== 6">{{ identityBusy ? '驗證中…' : '確認綁定 Email' }}</button>
            <button type="button" class="c-btn" :disabled="identityBusy || cooldown > 0" @click="sendLinkEmail">{{ cooldown > 0 ? `${cooldown} 秒後可重新寄送` : '重新寄送' }}</button>
            <button type="button" class="c-btn" :disabled="identityBusy" @click="changeLinkEmail">修改 Email</button>
          </template>
          <button v-else class="c-btn primary block" :disabled="identityBusy">{{ identityBusy ? '寄送中…' : '寄送綁定驗證碼' }}</button>
        </form>
        <p v-if="user.email" class="c-mute"><NuxtLink to="/forgot-password">設定／重設 Email 登入密碼</NuxtLink>（會登出所有裝置）</p>
        <p v-else class="c-mute">尚未綁定 Email，無法收取通知信或使用密碼重設。</p>
        <p v-if="identityErr && !removingProvider" class="c-err" role="alert">{{ identityErr }}</p>
        <p v-if="identityMsg" class="c-ok" role="status">{{ identityMsg }}</p>
      </section>
      <section class="c-card">
        <h2 class="c-h2">個人頁與捐助</h2>
        <form @submit.prevent="saveSocial">
          <label class="c-field"><span>帳號代號（個人頁網址，可留空）</span><input v-model="handle" maxlength="30" autocapitalize="none" autocomplete="off" placeholder="例如：amy_wang"></label>
          <p v-if="hBad" class="c-err" role="alert">小寫英文、數字、底線，3–30 字</p>
          <p v-if="profileUrl" class="c-mute">你的個人頁：<NuxtLink :to="`/u/${user.handle}`">{{ profileUrl }}</NuxtLink> <button type="button" class="c-btn" @click="copyUrl">{{ copied ? '已複製' : '複製' }}</button></p>
          <label class="c-field"><span>捐助紀錄預設誰看得到</span>
            <select v-model="dcv"><option value="public">公開</option><option value="friends">僅好友</option><option value="private">私人</option></select>
          </label>
          <p v-if="hErr" class="c-err" role="alert">{{ hErr }}</p>
          <p v-if="hMsg" class="c-ok" role="status">{{ hMsg }}</p>
          <button class="c-btn" :disabled="hBad">儲存</button>
        </form>
      </section>
      <section class="c-card">
        <h2 class="c-h2">通知</h2>
        <label class="c-switch"><input type="checkbox" :checked="!!user.email && user.notification_prefs?.email_claims" :disabled="!user.email" @change="toggle"><span>有人認領時寄 Email<br><small class="c-mute">第一筆即時通知，之後每日彙整一封。驚喜模式下只寫「有 N 件新認領」。</small></span></label>
      </section>
      <section class="c-card">
        <h2 class="c-h2">匯出資料</h2>
        <p class="c-mute">包含個人資料、清單與認領紀錄（JSON）。</p>
        <button class="c-btn" :disabled="exporting" @click="exportData">{{ exporting ? '匯出中…' : '匯出我的資料' }}</button>
      </section>
      <section class="c-card">
        <button class="c-btn block" @click="logout">登出</button>
      </section>
      <section class="c-card bad">
        <h2 class="c-h2">刪除帳號</h2>
        <p class="c-mute">我們會移除你的 Email 與名稱，你的清單會封存並從公開移除。此動作無法復原。</p>
        <button class="c-btn danger" @click="showDel = true; word = ''; delErr = ''">刪除我的帳號</button>
      </section>
      <p class="c-mute"><NuxtLink to="/terms">服務條款</NuxtLink>・<NuxtLink to="/privacy">隱私權政策</NuxtLink></p>
    </template>
    <CreatorConfirm :open="!!removingProvider" :title="`解除 ${removingLabel} 綁定？`" text="解除後會登出其他裝置，目前裝置保持登入。之後須使用其他已綁定的方式登入。沒有剩餘 Email 時無法收通知或重設密碼；日後重新驗證相同 Email 登入，仍可能自動連結回此帳號。" ok="解除綁定" danger :busy="identityBusy" :error="identityErr" @close="!identityBusy && (removingProvider = '')" @ok="unlinkIdentity" />
    <CreatorConfirm :open="showDel" title="確定要刪除帳號嗎？" text="你的 Email、名稱與登入方式會被移除，清單會封存並從公開移除，朋友無法再開啟。" ok="永久刪除" danger :busy="delBusy" :disabled="word !== '刪除'" :error="delErr" @close="showDel = false" @ok="remove">
      <label class="c-field"><span>請輸入「刪除」以確認</span><input v-model="word" autocomplete="off"></label>
    </CreatorConfirm>
  </main>
</template>

<style scoped>
.identity-list { list-style: none; padding: 0; margin: 0 0 1rem; }
.identity-list li { padding-block: .5rem; }
</style>
