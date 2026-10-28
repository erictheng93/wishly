<script setup lang="ts">
const i18n = useI18n()
useHead({ title: () => i18n.t('auth.register.title') })
const route = useRoute()
const { register, verifyRegister, oauth } = useAuth()
const redirect = safeRedirect(route.query.redirect)
const step = ref<'form' | 'otp'>('form')
const email = ref(''), password = ref(''), name = ref(''), code = ref('')
const otp = ref<{ focus: () => void }>()
const exists = ref(false)
const busy = ref(false), err = ref(''), wait = ref(0)
const challengeId = ref(''), needCaptcha = ref(false), tsToken = ref(''), capKey = ref(0)
// 請求過於頻繁時後端要求人機驗證：完成後自動重送
watch(tsToken, v => { if (v && needCaptcha.value && !busy.value) send() })
let t: any
function tick(n: number) { wait.value = n; clearInterval(t); t = setInterval(() => { if (--wait.value <= 0) clearInterval(t) }, 1000) }
onBeforeUnmount(() => clearInterval(t))

async function send() {
  err.value = ''; exists.value = false
  if (password.value.length < 8) { err.value = i18n.t('auth.register.pwShort'); return }
  busy.value = true
  try {
    const r = await register(email.value.trim(), password.value, name.value.trim(), tsToken.value || undefined)
    challengeId.value = r.challenge_id || ''; needCaptcha.value = false; tsToken.value = ''
    step.value = 'otp'; code.value = ''; tick(r.resend_after || 60)
    nextTick(() => otp.value?.focus())
  } catch (e: any) {
    if (e.code === 'CAPTCHA_REQUIRED') { needCaptcha.value = true; tsToken.value = ''; capKey.value++; err.value = i18n.t('auth.register.captcha') }
    else err.value = e.status === 429 ? i18n.t('auth.register.rate') : errMsg(e)
  } finally { busy.value = false }
}
async function verify() {
  if (code.value.length < 6 || busy.value) return
  err.value = ''; busy.value = true
  try {
    await verifyRegister(email.value.trim(), code.value, challengeId.value || undefined)
    await navigateTo(redirect)
  } catch (e: any) {
    if (e.code === 'EMAIL_EXISTS') { err.value = i18n.t('auth.register.exists'); exists.value = true }
    else { err.value = e.code === 'OTP_INVALID' ? i18n.t('auth.register.otpInvalid') : e.status === 429 ? i18n.t('auth.register.tooMany') : errMsg(e); code.value = ''; otp.value?.focus() }
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('auth.register.title')" back="/login" />
    <div class="c-auth">
      <form v-if="step === 'form'" @submit.prevent="send">
        <label class="c-field"><span>{{ $t('auth.register.displayName') }}</span><input v-model="name" type="text" autocomplete="nickname" maxlength="50" required></label>
        <label class="c-field"><span>Email</span><input v-model="email" type="email" autocomplete="email" placeholder="name@example.com" required></label>
        <CreatorPasswordField v-model="password" :label="$t('auth.register.password')" autocomplete="new-password" :hint="$t('auth.register.pwHint')" />
        <Turnstile v-if="needCaptcha" :key="capKey" v-model="tsToken" />
        <p v-if="err" class="c-err" role="alert">{{ err }}</p>
        <button class="c-btn primary block" :disabled="busy">{{ busy ? $t('auth.register.sending') : $t('auth.register.sendCode') }}</button>
      </form>
      <template v-if="step === 'form' && !isNative()">
        <p class="c-or">{{ $t('auth.register.or') }}</p>
        <button type="button" class="c-btn block c-mb" @click="oauth('google', redirect)">{{ $t('auth.register.google') }}</button>
        <button type="button" class="c-btn block" @click="oauth('line', redirect)">{{ $t('auth.register.line') }}</button>
      </template>
      <p v-if="step === 'form'" class="c-auth-foot">{{ $t('auth.register.haveAccount') }}<NuxtLink :to="{ path: '/login', query: route.query.redirect ? { redirect } : {} }">{{ $t('auth.register.login') }}</NuxtLink></p>
      <template v-if="step === 'otp'">
        <p>{{ $t('auth.register.codeSent', { email }) }}</p>
        <CreatorOtpInput ref="otp" v-model="code" @complete="verify" />
        <Turnstile v-if="needCaptcha" :key="capKey" v-model="tsToken" />
        <p v-if="err" class="c-err c-center" role="alert">{{ err }}</p>
        <NuxtLink v-if="exists" class="c-btn block c-mb" to="/login">{{ $t('auth.register.goLogin') }}</NuxtLink>
        <button class="c-btn primary block c-mb" :disabled="busy || code.length < 6" @click="verify">{{ busy ? $t('auth.register.verifying') : $t('auth.register.finish') }}</button>
        <p class="c-center c-mute">
          <button v-if="wait <= 0" class="c-btn" :disabled="busy" @click="send">{{ $t('auth.register.resend') }}</button>
          <span v-else>{{ $t('auth.register.resendIn', { n: wait }) }}</span>
        </p>
        <button class="c-btn block" @click="step = 'form'; err = ''">{{ $t('auth.register.edit') }}</button>
      </template>
    </div>
  </main>
</template>
