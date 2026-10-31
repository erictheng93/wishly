<script setup lang="ts">
// 營運人員兩步驟驗證（後台全面強制）：未啟用 → 掃 QR 設定並保存備用碼；已啟用 → 輸入驗證碼或備用碼解鎖此 session 的後台
definePageMeta({ middleware: 'staff' })
const { t } = useI18n()
useHead({ title: () => t('admin.mfa.title') })
const { api } = useApi()
const { user, fetchMe } = useAuth()
const route = useRoute()
const setup = ref<{ secret: string, otpauth_uri: string } | null>(null)
const code = ref(''), busy = ref(false), err = ref('')
const recovery = ref(false), codes = ref<string[] | null>(null)
const otp = ref<{ focus: () => void }>()
const enabled = computed(() => !!user.value?.mfa_enabled)
const next = computed(() => { const r = String(route.query.redirect || ''); return r.startsWith('/admin') && r !== '/admin/mfa' ? r : '/admin' })

async function start() {
  busy.value = true; err.value = ''
  try { setup.value = await api('/auth/mfa/totp/setup', { method: 'POST' }); nextTick(() => otp.value?.focus()) }
  catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
async function submit() {
  if (busy.value || (recovery.value ? !code.value.trim() : code.value.length !== 6)) return
  busy.value = true; err.value = ''
  try {
    const r = await api<any>(enabled.value ? '/auth/mfa/verify' : '/auth/mfa/totp/enable', { method: 'POST', body: { code: code.value } })
    await fetchMe(true)
    if (r?.recovery_codes) { codes.value = r.recovery_codes; return } // 剛啟用：先讓使用者抄下備用碼
    await navigateTo(next.value)
  } catch (e) { err.value = errMsg(e); code.value = ''; if (!recovery.value) otp.value?.focus() } finally { busy.value = false }
}
async function regen() {
  busy.value = true; err.value = ''
  try { codes.value = (await api<any>('/auth/mfa/recovery-codes', { method: 'POST' })).recovery_codes } catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
const toggleRecovery = () => { recovery.value = !recovery.value; code.value = ''; err.value = '' }
onMounted(() => { if (enabled.value) nextTick(() => otp.value?.focus()) })
</script>
<template>
  <main class="c-page mf">
    <CreatorHeader :title="$t('admin.mfa.title')" back="/dashboard" />
    <section class="c-card mf-card">
      <template v-if="codes">
        <p>{{ $t('admin.mfa.codesTitle') }}</p>
        <p class="c-mute">{{ $t('admin.mfa.codesHint') }}</p>
        <ol class="mfa-codes" data-testid="recovery-codes"><li v-for="c in codes" :key="c">{{ c }}</li></ol>
        <button class="c-btn primary block" @click="navigateTo(next)">{{ $t('admin.mfa.codesSaved') }}</button>
      </template>
      <template v-else-if="enabled && user?.mfa_verified">
        <p>{{ $t('admin.mfa.regenHint') }}</p>
        <button class="c-btn block" :disabled="busy" @click="regen">{{ $t('admin.mfa.regen') }}</button>
      </template>
      <template v-else-if="enabled">
        <p>{{ recovery ? $t('admin.mfa.recoveryPrompt') : $t('admin.mfa.enabledPrompt') }}</p>
      </template>
      <template v-else-if="!setup">
        <p>{{ $t('admin.mfa.intro') }}</p>
        <p class="c-mute">{{ $t('admin.mfa.lostPhone') }}</p>
        <button class="c-btn primary block" :disabled="busy" @click="start">{{ busy ? $t('admin.mfa.preparing') : $t('admin.mfa.start') }}</button>
      </template>
      <template v-else>
        <p>{{ $t('admin.mfa.step1') }}</p>
        <CreatorQr :text="setup.otpauth_uri" :size="180" :label="$t('admin.mfa.qrLabel')" :download="false" />
        <p class="c-mute mfa-secret">{{ setup.secret }}</p>
        <p>{{ $t('admin.mfa.step2') }}</p>
      </template>
      <form v-if="!codes && ((enabled && !user?.mfa_verified) || setup)" @submit.prevent="submit">
        <label v-if="recovery" class="c-field"><span>{{ $t('admin.mfa.recoveryLabel') }}</span><input v-model="code" name="recovery_code" autocomplete="one-time-code" autocapitalize="off" spellcheck="false"></label>
        <CreatorOtpInput v-else ref="otp" v-model="code" @complete="submit" />
        <button class="c-btn primary block" :disabled="busy || (recovery ? !code.trim() : code.length !== 6)">{{ busy ? $t('admin.mfa.verifying') : enabled ? $t('admin.mfa.verify') : $t('admin.mfa.enable') }}</button>
      </form>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <p v-if="!enabled && !user?.mfa_required" class="c-mute"><NuxtLink to="/admin">{{ $t('admin.mfa.skip') }}</NuxtLink></p>
      <p v-if="enabled && !user?.mfa_verified && !codes" class="c-mute"><button type="button" class="mf-link" @click="toggleRecovery">{{ recovery ? $t('admin.mfa.useApp') : $t('admin.mfa.useRecovery') }}</button></p>
    </section>
  </main>
</template>
<style scoped>
.mf-link{background:none;border:0;padding:0;min-height:44px;color:var(--fg);font:inherit;cursor:pointer;text-decoration:underline;text-decoration-color:var(--red);text-underline-offset:4px}
.mfa-codes{columns:2;font:600 15px var(--mono);letter-spacing:.08em;padding:12px 12px 12px 32px;border:1px dashed var(--ink-line);background:var(--bg-alt)}
.mfa-secret { font-family: var(--mono); word-break: break-all; text-align: center; padding: 10px; border: 1px dashed var(--ink-line); background: var(--bg-alt); letter-spacing: .12em; font-size: 13px; }
.mf-card{max-width:520px;margin:12px auto 0;padding:32px clamp(20px,5vw,40px);border:1px solid var(--ink-line);border-top:3px solid var(--red);box-shadow:8px 8px 0 var(--bg-alt),8px 8px 0 1px var(--line)}
.mf-card::before{content:"STAFF / 2FA" / "";display:block;margin-bottom:16px;font:600 11px var(--mono);letter-spacing:.18em;color:var(--mute)}
.mf-card .c-mute a{display:inline-flex;align-items:center;min-height:44px}
.mf-card>p:first-of-type{font-size:17px;font-weight:700;line-height:1.6}
</style>
