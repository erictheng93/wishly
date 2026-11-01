<script setup lang="ts">
// Email 驗證碼登入：綁定 Email 但沒設密碼的帳號靠這條回到帳號；信箱還沒有帳號時後端會直接開戶
const props = defineProps<{ email?: string }>()
const emit = defineEmits<{ done: [] }>()
const i18n = useI18n()
const { requestLoginCode, verifyLoginCode } = useAuth()
const step = ref<'email' | 'code'>('email')
const email = ref(props.email ?? ''), code = ref('')
const otp = ref<{ focus: () => void }>()
const busy = ref(false), err = ref(''), wait = ref(0)
const challengeId = ref(''), needCaptcha = ref(false), tsToken = ref(''), capKey = ref(0)
// 請求過於頻繁時後端要求人機驗證：完成後自動重送
watch(tsToken, v => { if (v && needCaptcha.value && !busy.value) send() })
let t: any
function tick(n: number) { wait.value = n; clearInterval(t); t = setInterval(() => { if (--wait.value <= 0) clearInterval(t) }, 1000) }
onBeforeUnmount(() => clearInterval(t))

async function send() {
  if (busy.value) return
  err.value = ''; busy.value = true
  try {
    const r = await requestLoginCode(email.value.trim(), tsToken.value || undefined)
    challengeId.value = r.challenge_id || ''; needCaptcha.value = false; tsToken.value = ''
    step.value = 'code'; code.value = ''; tick(r.resend_after || 60)
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
    await verifyLoginCode(email.value.trim(), code.value, challengeId.value || undefined)
    emit('done')
  } catch (e: any) {
    err.value = e.code === 'OTP_INVALID' ? i18n.t('auth.register.otpInvalid') : e.status === 429 ? i18n.t('auth.register.tooMany') : errMsg(e)
    code.value = ''; otp.value?.focus()
  } finally { busy.value = false }
}
function changeEmail() { step.value = 'email'; code.value = ''; challengeId.value = ''; err.value = '' }
</script>
<template>
  <form v-if="step === 'email'" @submit.prevent="send">
    <p class="c-mute">{{ $t('auth.login.codeNote') }}</p>
    <label class="c-field"><span>Email</span><input v-model="email" type="email" autocomplete="email" placeholder="name@example.com" required></label>
    <Turnstile v-if="needCaptcha" :key="capKey" v-model="tsToken" />
    <p v-if="err" class="c-err" role="alert">{{ err }} <SupportLink /></p>
    <button class="c-btn primary block" :disabled="busy">{{ busy ? $t('auth.register.sending') : $t('auth.register.sendCode') }}</button>
  </form>
  <template v-else>
    <p>{{ $t('auth.register.codeSent', { email: email.trim() }) }}</p>
    <CreatorOtpInput ref="otp" v-model="code" @complete="verify" />
    <Turnstile v-if="needCaptcha" :key="capKey" v-model="tsToken" />
    <p v-if="err" class="c-err c-center" role="alert">{{ err }} <SupportLink /></p>
    <button class="c-btn primary block c-mb" :disabled="busy || code.length < 6" @click="verify">{{ busy ? $t('auth.login.busy') : $t('auth.login.title') }}</button>
    <p class="c-center c-mute">
      <button v-if="wait <= 0" type="button" class="c-btn" :disabled="busy" @click="send">{{ $t('auth.register.resend') }}</button>
      <span v-else>{{ $t('auth.register.resendIn', { n: wait }) }}</span>
    </p>
    <button type="button" class="c-btn block" :disabled="busy" @click="changeEmail">{{ $t('auth.forgot.changeEmail') }}</button>
  </template>
</template>
