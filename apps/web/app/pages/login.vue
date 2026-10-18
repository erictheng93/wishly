<script setup lang="ts">
const { t } = useI18n()
useHead({ title: () => t('auth.login.title') })
const route = useRoute()
const { user, fetchMe, login, oauth } = useAuth()
const redirect = safeRedirect(route.query.redirect)
const email = ref(''), password = ref('')
const busy = ref(false)
const err = ref('')
const authError = useState<string>('auth-error', () => '')
const oauthErrors = (): Record<string, string> => ({
  oauth_failed: t('auth.login.oauthFailed'),
  oauth_unavailable: t('auth.login.oauthUnavailable'),
  account_blocked: t('auth.login.accountBlocked'),
  identity_in_use: t('auth.login.identityInUse'),
})
const loginError = computed(() => oauthErrors()[String(route.query.error || authError.value)] || '')
const okMsg = computed(() => route.query.reset ? t('auth.login.resetDone') : '')
const q = redirect === '/dashboard' ? {} : { redirect }
onMounted(async () => {
  if (route.query.error === 'account_blocked' || authError.value === 'account_blocked') {
    user.value = null
    setNativeToken(null)
    return
  }
  if (await fetchMe()) navigateTo(redirect)
})

async function submit() {
  err.value = ''; busy.value = true
  try {
    await login(email.value.trim(), password.value)
    await navigateTo(redirect)
  } catch (e: any) {
    err.value = e.code === 'INVALID_CREDENTIALS' ? t('auth.login.badCredentials') : e.status === 429 ? t('auth.login.tooMany') : errMsg(e)
  } finally { busy.value = false }
}

// 網頁版整頁導向供應商；原生殼等 deep link 回來後直接就緒
async function social(provider: 'google' | 'line') {
  err.value = ''; busy.value = true
  try {
    if (await oauth(provider, redirect)) await navigateTo(redirect)
  } catch (e: any) {
    if (e.message !== 'cancelled') err.value = oauthErrors()[e.message] || (e.code === 'NATIVE_CODE_INVALID' ? errMsg(e) : t('auth.login.oauthFailed'))
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('auth.login.title')" back="/" />
    <p v-if="okMsg" class="c-ok" role="status">{{ okMsg }}</p>
    <form @submit.prevent="submit">
      <label class="c-field"><span>Email</span><input v-model="email" type="email" autocomplete="email" placeholder="name@example.com" required></label>
      <CreatorPasswordField v-model="password" :label="$t('auth.login.password')" autocomplete="current-password" />
      <p v-if="err || loginError" class="c-err" role="alert">{{ err || loginError }}</p>
      <button class="c-btn primary block c-mb" :disabled="busy">{{ busy ? $t('auth.login.busy') : $t('auth.login.title') }}</button>
    </form>
    <div class="c-links">
      <NuxtLink :to="{ path: '/register', query: q }">{{ $t('auth.login.register') }}</NuxtLink>
      <NuxtLink to="/forgot-password">{{ $t('auth.login.forgot') }}</NuxtLink>
    </div>
    <p class="c-center c-mute">{{ $t('auth.login.or') }}</p>
    <button type="button" class="c-btn block c-mb" :disabled="busy" @click="social('google')">{{ $t('auth.login.google') }}</button>
    <button type="button" class="c-btn block c-mb" :disabled="busy" @click="social('line')">{{ $t('auth.login.line') }}</button>
    <p class="c-mute">{{ $t('auth.login.note') }}</p>
  </main>
</template>
