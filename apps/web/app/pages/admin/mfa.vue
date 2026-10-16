<script setup lang="ts">
// 營運人員兩步驟驗證：未啟用 → 掃 QR 設定；已啟用 → 輸入驗證碼解鎖此 session 的後台
definePageMeta({ middleware: 'staff' })
useHead({ title: '兩步驟驗證' })
const { api } = useApi()
const { user, fetchMe } = useAuth()
const route = useRoute()
const setup = ref<{ secret: string, otpauth_uri: string } | null>(null)
const code = ref(''), busy = ref(false), err = ref('')
const otp = ref<{ focus: () => void }>()
const enabled = computed(() => !!user.value?.mfa_enabled)
const next = computed(() => { const r = String(route.query.redirect || ''); return r.startsWith('/admin') && r !== '/admin/mfa' ? r : '/admin' })

async function start() {
  busy.value = true; err.value = ''
  try { setup.value = await api('/auth/mfa/totp/setup', { method: 'POST' }); nextTick(() => otp.value?.focus()) }
  catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
async function submit() {
  if (busy.value || code.value.length !== 6) return
  busy.value = true; err.value = ''
  try {
    await api(enabled.value ? '/auth/mfa/verify' : '/auth/mfa/totp/enable', { method: 'POST', body: { code: code.value } })
    await fetchMe(true)
    await navigateTo(next.value)
  } catch (e) { err.value = errMsg(e); code.value = ''; otp.value?.focus() } finally { busy.value = false }
}
onMounted(() => { if (enabled.value) nextTick(() => otp.value?.focus()) })
</script>
<template>
  <main class="c-page">
    <CreatorHeader title="兩步驟驗證" back="/dashboard" />
    <section class="c-card">
      <template v-if="enabled">
        <p>請輸入驗證器 App 顯示的 6 位數驗證碼。</p>
      </template>
      <template v-else-if="!setup">
        <p>啟用後，每次登入後台都要再輸入驗證器 App（Google Authenticator、1Password 等）上的 6 位數驗證碼。</p>
        <p class="c-mute">手機遺失時，可請其他營運人員在後台「使用者」幫你重設。</p>
        <button class="c-btn primary block" :disabled="busy" @click="start">{{ busy ? '準備中…' : '開始設定' }}</button>
      </template>
      <template v-else>
        <p>1. 用驗證器 App 掃描 QR code，或手動輸入金鑰：</p>
        <CreatorQr :text="setup.otpauth_uri" :size="180" label="兩步驟驗證設定 QR code" :download="false" />
        <p class="c-mute mfa-secret">{{ setup.secret }}</p>
        <p>2. 輸入 App 顯示的 6 位數驗證碼完成啟用。</p>
      </template>
      <form v-if="enabled || setup" @submit.prevent="submit">
        <CreatorOtpInput ref="otp" v-model="code" @complete="submit" />
        <button class="c-btn primary block" :disabled="busy || code.length !== 6">{{ busy ? '驗證中…' : enabled ? '驗證' : '啟用' }}</button>
      </form>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <p v-if="!enabled" class="c-mute"><NuxtLink to="/admin">先略過，回後台</NuxtLink></p>
    </section>
  </main>
</template>
<style scoped>
.mfa-secret { font-family: ui-monospace, monospace; word-break: break-all; text-align: center; }
</style>
