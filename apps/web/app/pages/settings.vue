<script setup lang="ts">
import '~/assets/creator-pages.css'
definePageMeta({ middleware: 'auth' })
const { t, locale } = useI18n()
useHead({ title: () => t('settings.title') })
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
const handle = ref(''), dcv = ref('friends'), hMsg = ref(''), hErr = ref(''), copied = ref(false), idCopied = ref(false)
const profileUrl = computed(() => user.value?.handle ? `${siteOrigin()}/u/${user.value.handle}` : '')
const hBad = computed(() => !!handle.value && !isValidHandle(handle.value))
const bio = ref(''), bMonth = ref(''), bDay = ref(''), bVis = ref('friends')
const bDays = computed(() => daysInMonth(bMonth.value))
const pad = (n: number) => String(n).padStart(2, '0')
const exporting = ref(false), exportErr = ref('')
const showDel = ref(false), word = ref(''), delPw = ref(''), delBusy = ref(false), delErr = ref(''), delReauth = ref(false)
const LINK_ERROR_KEYS = ['oauth_failed', 'oauth_unavailable', 'identity_in_use', 'provider_already_linked']
const linkError = (code: string) => (LINK_ERROR_KEYS.includes(code) ? t(`settings.login.err.${code}`) : '')
// 網頁版整頁導向供應商再回來；原生殼等 deep link 回來後就地更新
async function linkOAuth(p: (typeof providers)[number]) {
  if (identityBusy.value || p.provider === 'email') return
  identityBusy.value = true; identityErr.value = identityMsg.value = ''
  try {
    await oauth(p.provider, '/settings', true)
    if (isNative()) identityMsg.value = t('settings.login.linkedMsg', { label: p.label })
  } catch (e: any) {
    if (e.message !== 'cancelled') identityErr.value = linkError(e.message) || errMsg(e)
  } finally { identityBusy.value = false }
}
onMounted(async () => { await fetchMe(true); name.value = user.value?.display_name ?? ''; handle.value = user.value?.handle ?? ''; dcv.value = user.value?.default_claim_visibility ?? 'friends'
  bio.value = user.value?.bio ?? ''; [bMonth.value = '', bDay.value = ''] = user.value?.birthday?.split('-') ?? []; bVis.value = user.value?.birthday_visibility ?? 'friends'; linkEmail.value = user.value?.email ?? ''
  identityErr.value = linkError(String(route.query.error))
  const label = providers.find(p => p.provider === route.query.linked)?.label
  if (label) identityMsg.value = t('settings.login.linkedMsg', { label })
  loadSessions()
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
    identityMsg.value = t('settings.login.emailLinkedMsg')
  } catch (e) { identityErr.value = errMsg(e); linkCode.value = ''; linkOtp.value?.focus() } finally { identityBusy.value = false }
}
function closeUnlink() { if (!identityBusy.value) removingProvider.value = '' }
async function unlinkIdentity() {
  if (identityBusy.value) return
  identityBusy.value = true; identityErr.value = identityMsg.value = ''
  try {
    user.value = await api(`/me/identities/${removingProvider.value}`, { method: 'DELETE' })
    identityMsg.value = t('settings.login.unlinkedMsg', { label: removingLabel.value })
    removingProvider.value = ''; linkEmail.value = user.value.email ?? ''
  } catch (e) { identityErr.value = errMsg(e) } finally { identityBusy.value = false }
}

// 登入裝置
const sessions = ref<any[]>([]), sessBusy = ref(false), sessErr = ref(''), sessMsg = ref('')
const OS: [RegExp, string][] = [[/iPhone|iPad/, 'iOS'], [/Android/, 'Android'], [/Mac OS/, 'macOS'], [/Windows/, 'Windows'], [/Linux/, 'Linux']]
const BROWSER: [RegExp, string][] = [[/Edg\//, 'Edge'], [/Chrome\//, 'Chrome'], [/Firefox\//, 'Firefox'], [/Safari\//, 'Safari']]
function device(ua?: string | null) {
  if (!ua) return t('settings.sess.unknown')
  const pick = (t: [RegExp, string][]) => t.find(([r]) => r.test(ua))?.[1]
  return [pick(BROWSER), pick(OS)].filter(Boolean).join('・') || ua.slice(0, 60)
}
const when = (d: string) => new Date(d).toLocaleString(locale.value === 'zh-Hant' ? 'zh-TW' : locale.value, { dateStyle: 'medium', timeStyle: 'short' })
async function loadSessions() {
  try { sessions.value = (await api('/me/sessions')).data } catch (e) { sessErr.value = errMsg(e) }
}
async function revokeSession(id?: string) {
  if (sessBusy.value) return
  sessBusy.value = true; sessErr.value = sessMsg.value = ''
  try {
    if (id) { await api(`/me/sessions/${id}`, { method: 'DELETE' }); sessMsg.value = t('settings.sess.loggedOutOne') }
    else { const r = await api('/me/sessions', { method: 'DELETE' }); sessMsg.value = t('settings.sess.loggedOutN', { n: r.revoked }) }
    await loadSessions()
  } catch (e) { sessErr.value = errMsg(e) } finally { sessBusy.value = false }
}
// 變更密碼（需目前密碼；沒有密碼者走忘記密碼流程設定）
const curPw = ref(''), newPw = ref(''), pwBusy = ref(false), pwErr = ref(''), pwMsg = ref('')
async function changePassword() {
  if (pwBusy.value) return
  pwBusy.value = true; pwErr.value = pwMsg.value = ''
  try {
    await api('/me/password', { method: 'POST', body: { current_password: curPw.value, new_password: newPw.value } })
    curPw.value = newPw.value = ''; pwMsg.value = t('settings.pw.changed')
    await loadSessions()
  } catch (e) { pwErr.value = errMsg(e) } finally { pwBusy.value = false }
}

async function saveName() {
  msg.value = err.value = ''
  try { user.value = await api('/me', { method: 'PATCH', body: { display_name: name.value.trim() } }); msg.value = t('settings.saved') }
  catch (e) { err.value = errMsg(e) }
}
const nBusy = ref(false), nErr = ref('')
async function setMailPref(key: 'email_gifts' | 'email_social', on: boolean) {
  nBusy.value = true; nErr.value = ''
  try { user.value = await api('/me', { method: 'PATCH', body: { notification_prefs: { [key]: on } } }) } catch (e: any) { nErr.value = apiErrMsg(e) } finally { nBusy.value = false }
}
async function saveSocial() {
  hMsg.value = hErr.value = ''
  if (hBad.value) { hErr.value = t('settings.social.handleBadErr'); return }
  const birthday = toBirthday(bMonth.value, bDay.value)
  if (birthday === undefined) { hErr.value = t('settings.social.birthdayErr'); return }
  try {
    user.value = await api('/me', { method: 'PATCH', body: {
      handle: handle.value || null, default_claim_visibility: dcv.value,
      bio: bio.value.trim() || null, birthday, birthday_visibility: bVis.value } })
    ;[bMonth.value = '', bDay.value = ''] = user.value.birthday?.split('-') ?? [] // 日被夾到月底時同步顯示
    hMsg.value = t('settings.saved')
  } catch (e: any) { hErr.value = apiErrMsg(e) }
}
async function copyId() { try { await copyText(user.value!.id); idCopied.value = true } catch { msg.value = t('settings.social.copyFail') } }
async function copyUrl() { try { await copyText(profileUrl.value); copied.value = true } catch { hErr.value = t('settings.social.copyFail') } }
// 新手導覽：重新開始後帶到導覽頁
const tourBusy = ref(false), tourErr = ref('')
async function restartTour() {
  tourBusy.value = true; tourErr.value = ''
  try { await useOnboarding().finish(false); await navigateTo('/welcome') } catch (e) { tourErr.value = errMsg(e) } finally { tourBusy.value = false }
}
async function exportData() {
  exporting.value = true; exportErr.value = ''
  try {
    const data = await api('/me/export')
    const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' }))
    const a = Object.assign(document.createElement('a'), { href: url, download: `wishly-export-${new Date().toISOString().slice(0, 10)}.json` })
    a.click(); URL.revokeObjectURL(url)
  } catch (e) { exportErr.value = errMsg(e) } finally { exporting.value = false }
}
async function remove() {
  delBusy.value = true; delErr.value = ''; delReauth.value = false
  try {
    await api('/me', { method: 'DELETE', body: { confirm: 'DELETE', ...(user.value?.has_password ? { password: delPw.value } : {}) } })
    user.value = null
    await navigateTo('/')
  } catch (e: any) {
    delReauth.value = e.code === 'REAUTH_REQUIRED'
    delErr.value = e.code === 'INVALID_CREDENTIALS' ? t('settings.del.badPassword') : e.code === 'ACCOUNT_HAS_POINTS' ? t('settings.del.hasPoints', { detail: e.detail }) : e.code === 'ACCOUNT_HAS_BALANCE' ? t('settings.del.hasBalance', { detail: e.detail, balance: e.body.balance ?? 0, pledged: e.body.pledged_count ?? 0, orders: e.body.open_order_count ?? 0 }) : errMsg(e)
  } finally { delBusy.value = false }
}
</script>
<template>
  <main class="c-page x-set">
    <CreatorHeader :title="$t('settings.title')" back="/dashboard" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <template v-if="user">
      <section class="c-card">
        <h2 class="c-h2">{{ $t('settings.profile') }}</h2>
        <p class="c-mute">{{ user.email || $t('settings.noEmail') }}</p>
        <p class="c-mute st-kv"><span>{{ $t('settings.accountId') }}</span><code>{{ user.id }}</code> <button type="button" class="c-btn" @click="copyId">{{ idCopied ? $t('settings.social.copied') : $t('settings.social.copy') }}</button></p>
        <p class="c-mute">{{ $t('common.support') }}<SupportLink /></p>
        <form @submit.prevent="saveName">
          <label class="c-field"><span>{{ $t('settings.displayName') }}</span><input v-model="name" maxlength="40" required></label>
          <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
          <button class="c-btn">{{ $t('settings.save') }}</button>
        </form>
      </section>
      <section class="c-card" aria-labelledby="login-methods-title">
        <h2 id="login-methods-title" class="c-h2">{{ $t('settings.login.title') }}</h2>
        <p class="c-mute">{{ $t('settings.login.hint') }}</p>
        <ul class="identity-list">
          <li v-for="p in providers" :key="p.provider" class="c-row">
            <span class="c-grow">{{ p.label }} <small class="c-mute">{{ linked.has(p.provider) ? $t('settings.login.linked') : $t('settings.login.unlinked') }}</small></span>
            <button v-if="linked.has(p.provider)" type="button" class="c-btn" :aria-label="$t('settings.login.unlinkAria', { label: p.label })" :disabled="identityBusy || linked.size <= 1" @click="removingProvider = p.provider; identityErr = ''">{{ $t('settings.login.unlink') }}</button>
            <button v-else-if="p.provider !== 'email'" type="button" class="c-btn" :disabled="identityBusy" @click="linkOAuth(p)">{{ $t('settings.login.link', { label: p.label }) }}</button>
          </li>
        </ul>
        <p v-if="linked.size <= 1" class="c-mute">{{ $t('settings.login.keepOne') }}</p>
        <form v-if="!linked.has('email')" @submit.prevent="sentEmail ? verifyLinkEmail() : sendLinkEmail()">
          <label class="c-field"><span>{{ $t('settings.login.linkEmail') }}</span><input v-model="linkEmail" type="email" autocomplete="email" maxlength="254" :disabled="!!sentEmail || identityBusy" required></label>
          <template v-if="sentEmail">
            <p class="c-mute">{{ $t('settings.login.codeSent', { email: sentEmail }) }}</p>
            <CreatorOtpInput ref="linkOtp" v-model="linkCode" @complete="verifyLinkEmail" />
            <button class="c-btn primary block c-mb" :disabled="identityBusy || linkCode.length !== 6">{{ identityBusy ? $t('settings.login.verifying') : $t('settings.login.confirmEmail') }}</button>
            <button type="button" class="c-btn" :disabled="identityBusy || cooldown > 0" @click="sendLinkEmail">{{ cooldown > 0 ? $t('settings.login.resendIn', { n: cooldown }) : $t('settings.login.resend') }}</button>
            <button type="button" class="c-btn" :disabled="identityBusy" @click="changeLinkEmail">{{ $t('settings.login.changeEmail') }}</button>
          </template>
          <button v-else class="c-btn primary block" :disabled="identityBusy">{{ identityBusy ? $t('settings.login.sending') : $t('settings.login.sendCode') }}</button>
        </form>
        <p v-if="user.email && !user.has_password" class="c-mute"><NuxtLink to="/forgot-password">{{ $t('settings.login.setPw') }}</NuxtLink>{{ $t('settings.login.setPwNote') }}</p>
        <p v-else-if="!user.email" class="c-mute">{{ $t('settings.login.noEmailWarn') }}</p>
        <p v-if="identityErr && !removingProvider" class="c-err" role="alert">{{ identityErr }}</p>
        <p v-if="identityMsg" class="c-ok" role="status">{{ identityMsg }}</p>
      </section>
      <section v-if="user.has_password" class="c-card">
        <h2 class="c-h2">{{ $t('settings.pw.title') }}</h2>
        <form @submit.prevent="changePassword">
          <CreatorPasswordField v-model="curPw" :label="$t('settings.pw.current')" autocomplete="current-password" />
          <CreatorPasswordField v-model="newPw" :label="$t('settings.pw.new')" autocomplete="new-password" :hint="$t('settings.pw.hint')" />
          <p v-if="pwErr" class="c-err" role="alert">{{ pwErr }}</p>
          <p v-if="pwMsg" class="c-ok" role="status">{{ pwMsg }}</p>
          <button class="c-btn" :disabled="pwBusy">{{ pwBusy ? $t('settings.pw.changing') : $t('settings.pw.title') }}</button>
        </form>
        <p class="c-mute"><NuxtLink to="/forgot-password">{{ $t('settings.pw.forgot') }}</NuxtLink></p>
      </section>
      <section class="c-card" aria-labelledby="sessions-title">
        <h2 id="sessions-title" class="c-h2">{{ $t('settings.sess.title') }}</h2>
        <ul class="identity-list">
          <li v-for="s in sessions" :key="s.id" class="c-row">
            <span class="c-grow">{{ device(s.user_agent) }} <small v-if="s.current" class="c-badge">{{ $t('settings.sess.current') }}</small><br>
              <small class="c-mute">{{ $t('settings.sess.lastUsed', { time: when(s.last_seen_at) }) }}<template v-if="s.ip">・{{ s.ip }}</template>{{ $t('settings.sess.signedInAt', { time: when(s.created_at) }) }}</small></span>
            <button v-if="!s.current" type="button" class="c-btn" :disabled="sessBusy" @click="revokeSession(s.id)">{{ $t('settings.sess.logout') }}</button>
          </li>
        </ul>
        <button v-if="sessions.length > 1" type="button" class="c-btn" :disabled="sessBusy" @click="revokeSession()">{{ $t('settings.sess.logoutOthers') }}</button>
        <p v-if="sessErr" class="c-err" role="alert">{{ sessErr }}</p>
        <p v-if="sessMsg" class="c-ok" role="status">{{ sessMsg }}</p>
      </section>
      <section class="c-card">
        <h2 class="c-h2">{{ $t('settings.social.title') }}</h2>
        <form @submit.prevent="saveSocial">
          <label class="c-field"><span>{{ $t('settings.social.handle') }}</span><input v-model="handle" maxlength="30" autocapitalize="none" autocomplete="off" :placeholder="$t('settings.social.handlePh')"></label>
          <p v-if="hBad" class="c-err" role="alert">{{ $t('settings.social.handleBadHint') }}</p>
          <p v-if="profileUrl" class="c-mute st-kv"><span>{{ $t('settings.social.yourPage') }}</span><NuxtLink :to="`/u/${user.handle}`">{{ profileUrl }}</NuxtLink> <button type="button" class="c-btn" @click="copyUrl">{{ copied ? $t('settings.social.copied') : $t('settings.social.copy') }}</button></p>
          <label class="c-field"><span>{{ $t('settings.social.bio') }}</span><textarea v-model="bio" maxlength="160" rows="3" /></label>
          <div class="c-field" role="group" aria-labelledby="bday-label"><span id="bday-label">{{ $t('settings.social.birthday') }}</span>
            <div class="c-row">
              <select v-model="bMonth" :aria-label="$t('settings.social.monthAria')"><option value="">{{ $t('settings.social.monthOpt') }}</option><option v-for="m in 12" :key="m" :value="pad(m)">{{ $t('settings.social.monthN', { n: m }) }}</option></select>
              <select v-model="bDay" :aria-label="$t('settings.social.dayAria')"><option value="">{{ $t('settings.social.dayOpt') }}</option><option v-for="d in bDays" :key="d" :value="pad(d)">{{ $t('settings.social.dayN', { n: d }) }}</option></select>
            </div>
          </div>
          <label class="c-field"><span>{{ $t('settings.social.birthdayVis') }}</span>
            <select v-model="bVis"><option value="public">{{ $t('settings.vis.public') }}</option><option value="friends">{{ $t('settings.vis.friends') }}</option><option value="private">{{ $t('settings.social.onlyMe') }}</option></select>
          </label>
          <label class="c-field"><span>{{ $t('settings.social.claimVis') }}</span>
            <select v-model="dcv"><option value="public">{{ $t('settings.vis.public') }}</option><option value="friends">{{ $t('settings.vis.friends') }}</option><option value="private">{{ $t('settings.vis.private') }}</option></select>
          </label>
          <p v-if="hErr" class="c-err" role="alert">{{ hErr }}</p>
          <p v-if="hMsg" class="c-ok" role="status">{{ hMsg }}</p>
          <button class="c-btn" :disabled="hBad">{{ $t('settings.save') }}</button>
        </form>
      </section>
      <section class="c-card">
        <h2 class="c-h2">{{ $t('settings.notify.title') }}</h2>
        <label class="c-switch"><input type="checkbox" checked disabled><span>{{ $t('settings.notify.account') }}<br><small class="c-mute">{{ $t('settings.notify.accountHint') }}</small></span></label>
        <label v-for="k in (['gifts', 'social'] as const)" :key="k" class="c-switch"><input type="checkbox" :checked="user?.notification_prefs?.[`email_${k}`] !== false" :disabled="nBusy" @change="setMailPref(`email_${k}`, ($event.target as HTMLInputElement).checked)"><span>{{ $t(`settings.notify.${k}`) }}<br><small class="c-mute">{{ $t(`settings.notify.${k}Hint`) }}</small></span></label>
        <p v-if="nErr" class="c-err" role="alert">{{ nErr }}</p>
      </section>
      <section class="c-card">
        <h2 class="c-h2">{{ $t('onboarding.settings.title') }}</h2>
        <p class="c-mute">{{ $t('onboarding.settings.hint') }}</p>
        <p v-if="tourErr" class="c-err" role="alert">{{ tourErr }}</p>
        <button type="button" class="c-btn" :disabled="tourBusy" @click="restartTour">{{ $t('onboarding.settings.restart') }}</button>
      </section>
      <section class="c-card">
        <h2 class="c-h2">{{ $t('settings.export.title') }}</h2>
        <p class="c-mute">{{ $t('settings.export.hint') }}</p>
        <p v-if="exportErr" class="c-err" role="alert">{{ exportErr }}</p><!-- 放在按鈕上方：手機上按鈕常在畫面底緣，訊息才不會落到畫面外 -->
        <button class="c-btn" :disabled="exporting" @click="exportData">{{ exporting ? $t('settings.export.busy') : $t('settings.export.btn') }}</button>
      </section>
      <section class="c-card">
        <button class="c-btn block" @click="logout()">{{ $t('settings.logout') }}</button>
      </section>
      <section class="c-card bad">
        <h2 class="c-h2">{{ $t('settings.del.title') }}</h2>
        <p class="c-mute">{{ $t('settings.del.text') }}</p>
        <button class="c-btn danger" @click="showDel = true; word = ''; delPw = ''; delErr = ''; delReauth = false">{{ $t('settings.del.btn') }}</button>
      </section>
      <p class="c-mute"><NuxtLink to="/terms">{{ $t('settings.terms') }}</NuxtLink>・<NuxtLink to="/privacy">{{ $t('settings.privacy') }}</NuxtLink></p>
    </template>
    <CreatorConfirm :open="!!removingProvider" :title="$t('settings.login.unlinkTitle', { label: removingLabel })" :text="$t('settings.login.unlinkText')" :ok="$t('settings.login.unlinkOk')" danger :busy="identityBusy" :error="identityErr" @close="!identityBusy && (removingProvider = '')" @ok="unlinkIdentity" />
    <CreatorConfirm :open="showDel" :title="$t('settings.del.confirmTitle')" :text="$t('settings.del.confirmText')" :ok="$t('settings.del.ok')" danger :busy="delBusy" :disabled="word !== $t('settings.del.word') || (user?.has_password && !delPw)" :error="delErr" @close="showDel = false" @ok="remove">
      <label class="c-field"><span>{{ $t('settings.del.prompt', { word: $t('settings.del.word') }) }}</span><input v-model="word" autocomplete="off"></label>
      <CreatorPasswordField v-if="user?.has_password" v-model="delPw" :label="$t('settings.del.password')" autocomplete="current-password" />
      <button v-if="delReauth" type="button" class="c-btn primary block c-mb" @click="logout('/settings')">{{ $t('settings.del.relogin') }}</button>
    </CreatorConfirm>
  </main>
</template>

<style scoped>
.st-kv{display:flex;flex-wrap:wrap;align-items:center;gap:2px 12px}
.st-kv>span{flex:0 0 100%;font:600 11px/1.6 var(--mono);letter-spacing:.1em}
.st-kv>code,.st-kv>a{flex:1 1 12em;min-width:0;overflow-wrap:anywhere;font:500 13px/1.5 var(--mono);color:var(--fg)}
.identity-list { list-style: none; padding: 0; margin: 0 0 1rem; }
.identity-list li { padding-block: .5rem; border-bottom: 1px solid var(--line); }
.identity-list li:last-child { border-bottom: 0; }
/* 編輯式設定頁：無框區塊，墨色頂線；桌面為「標題欄 ／ 內容欄」兩欄 */
.x-set { counter-reset: sec; }
.x-set > .c-card { counter-increment: sec; border: 0; border-top: 1px solid var(--ink-line); background: transparent; padding: 22px 0 28px; margin: 0; }
.x-set > .c-card:first-of-type { border-top: 0; padding-top: 6px; } /* 標題下已有墨線，第一區不再重複 */
.x-set > .c-card > h2:first-child { display: block; font-size: 20px; margin: 0 0 16px; }
.x-set > .c-card > h2:first-child::before { content: counter(sec, decimal-leading-zero) / ""; display: block; width: auto; height: auto; background: none; margin-bottom: 8px; font: 600 11px/1 var(--mono); letter-spacing: .14em; color: var(--red-text); }
.x-set > .c-card.bad { background: var(--bad-bg); border-top-color: var(--bad); padding-inline: 20px; box-shadow: none; margin-top: 8px; }
.x-set > .c-card.bad > h2:first-child { color: var(--bad); }
.x-set > .c-card:has(> .c-btn.block) { counter-increment: none; }
.x-set > .c-card > p.c-mute:first-of-type { margin-top: 0; }
@media (min-width: 768px) {
  .x-set > .c-card { display: grid; grid-template-columns: 200px minmax(0, 1fr); column-gap: 40px; align-items: start; }
  .x-set > .c-card > h2:first-child { grid-column: 1; grid-row: 1 / span 30; margin: 0; }
  .x-set > .c-card > :not(h2:first-child) { grid-column: 2; }
  .x-set > .c-card > .c-btn:not(.block) { justify-self: start; }
  .x-set > .c-card:not(:has(> h2)) { display: block; }
  .x-set > .c-card.bad { padding-inline: 24px; }
}
.x-set .c-field input, .x-set .c-field select, .x-set .c-field textarea { max-width: 480px; }

</style>
