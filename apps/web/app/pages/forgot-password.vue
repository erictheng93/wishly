<script setup lang="ts">
const i18n = useI18n()
useHead({ title: () => i18n.t('auth.forgot.title') })
const { requestReset, confirmReset } = useAuth()
const step = ref<'email' | 'reset'>('email')
const email = ref(''), password = ref(''), code = ref('')
const otp = ref<{ focus: () => void }>()
const busy = ref(false), err = ref(''), wait = ref(0)
const challengeId = ref(''), needCaptcha = ref(false), tsToken = ref(''), capKey = ref(0)
// 請求過於頻繁時後端要求人機驗證：完成後自動重送
watch(tsToken, v => { if (v && needCaptcha.value && !busy.value) send() })
let t: any
function tick(n: number) { wait.value = n; clearInterval(t); t = setInterval(() => { if (--wait.value <= 0) clearInterval(t) }, 1000) }
onBeforeUnmount(() => clearInterval(t))

async function send() {
  err.value = ''; busy.value = true
  try {
    const r = await requestReset(email.value.trim(), tsToken.value || undefined)
    challengeId.value = r.challenge_id || ''; needCaptcha.value = false; tsToken.value = ''
    step.value = 'reset'; tick(r.resend_after || 60)
  } catch (e: any) {
    if (e.code === 'CAPTCHA_REQUIRED') { needCaptcha.value = true; tsToken.value = ''; capKey.value++; err.value = i18n.t('auth.forgot.captcha') }
    else err.value = e.status === 429 ? i18n.t('auth.forgot.rate') : errMsg(e)
  } finally { busy.value = false }
}
function changeEmail() { step.value = 'email'; code.value = ''; challengeId.value = ''; err.value = '' }
async function confirm() {
  err.value = ''
  if (code.value.length < 6) { err.value = i18n.t('auth.forgot.codeLen'); return }
  if (password.value.length < 8) { err.value = i18n.t('auth.forgot.pwShort'); return }
  busy.value = true
  try {
    await confirmReset(email.value.trim(), code.value, password.value, challengeId.value || undefined)
    await navigateTo({ path: '/login', query: { reset: '1' } })
  } catch (e: any) {
    err.value = e.code === 'OTP_INVALID' ? i18n.t('auth.forgot.otpInvalid') : e.status === 429 ? i18n.t('auth.forgot.tooMany') : errMsg(e)
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('auth.forgot.title')" back="/login" />
    <div class="c-auth">
      <form v-if="step === 'email'" @submit.prevent="send">
        <label class="c-field"><span>Email</span><input v-model="email" type="email" autocomplete="email" placeholder="name@example.com" required></label>
        <Turnstile v-if="needCaptcha" :key="capKey" v-model="tsToken" />
        <p v-if="err" class="c-err" role="alert">{{ err }}</p>
        <button class="c-btn primary block" :disabled="busy">{{ busy ? $t('auth.forgot.sending') : $t('auth.forgot.sendCode') }}</button>
      </form>
      <form v-else @submit.prevent="confirm">
        <p class="c-ok" role="status">{{ $t('auth.forgot.sentNote') }}</p>
        <span class="c-lbl">{{ $t('auth.forgot.code') }}</span>
        <CreatorOtpInput ref="otp" v-model="code" />
        <CreatorPasswordField v-model="password" :label="$t('auth.forgot.newPassword')" autocomplete="new-password" :hint="$t('auth.forgot.pwHint')" />
        <Turnstile v-if="needCaptcha" :key="capKey" v-model="tsToken" />
        <p v-if="err" class="c-err" role="alert">{{ err }}</p>
        <button class="c-btn primary block c-mb" :disabled="busy">{{ busy ? $t('auth.forgot.resetting') : $t('auth.forgot.reset') }}</button>
        <p class="c-center c-mute">
          <button v-if="wait <= 0" type="button" class="c-btn" :disabled="busy" @click="send">{{ $t('auth.forgot.resend') }}</button>
          <span v-else>{{ $t('auth.forgot.resendIn', { n: wait }) }}</span>
        </p>
        <p class="c-center"><button type="button" class="c-btn" :disabled="busy" @click="changeEmail">{{ $t('auth.forgot.changeEmail') }}</button></p>
      </form>
    </div>
  </main>
</template>
