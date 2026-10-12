<script setup lang="ts">
useHead({ title: '登入' })
const route = useRoute()
const { user, fetchMe, login, oauth } = useAuth()
const raw = String(route.query.redirect || '')
const redirect = raw.startsWith('/') && !raw.startsWith('//') ? raw : '/dashboard'
const email = ref(''), password = ref('')
const busy = ref(false)
const err = ref('')
const authError = useState<string>('auth-error', () => '')
const loginError = computed(() => ({
  oauth_failed: '第三方登入失敗，請重試',
  oauth_unavailable: '此登入方式尚未開放',
  account_blocked: '帳號已停權，請聯絡客服',
  identity_in_use: '此登入方式或 Email 已屬於另一個帳號，請使用原帳號登入',
}[String(route.query.error || authError.value)] || ''))
const okMsg = route.query.reset ? '密碼已重設，請用新密碼登入' : ''
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
    err.value = e.code === 'INVALID_CREDENTIALS' ? '帳號或密碼錯誤' : e.status === 429 ? '嘗試次數過多，請稍後再試' : errMsg(e)
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader title="登入" back="/" />
    <p v-if="okMsg" class="c-ok" role="status">{{ okMsg }}</p>
    <form @submit.prevent="submit">
      <label class="c-field"><span>Email</span><input v-model="email" type="email" autocomplete="email" placeholder="name@example.com" required></label>
      <CreatorPasswordField v-model="password" label="密碼" autocomplete="current-password" />
      <p v-if="err || loginError" class="c-err" role="alert">{{ err || loginError }}</p>
      <button class="c-btn primary block c-mb" :disabled="busy">{{ busy ? '登入中…' : '登入' }}</button>
    </form>
    <div class="c-links">
      <NuxtLink :to="{ path: '/register', query: q }">建立帳號</NuxtLink>
      <NuxtLink to="/forgot-password">忘記密碼</NuxtLink>
    </div>
    <p v-if="!isNative()" class="c-center c-mute">或</p>
    <button v-if="!isNative()" type="button" class="c-btn block c-mb" @click="oauth('google', redirect)">使用 Google 繼續</button>
    <button v-if="!isNative()" type="button" class="c-btn block c-mb" @click="oauth('line', redirect)">使用 LINE 繼續</button>
    <p class="c-mute">送禮的朋友不需要登入，直接用分享連結即可。</p>
  </main>
</template>
