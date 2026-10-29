<script setup lang="ts">
// 營運人員兩步驟驗證：未啟用 → 掃 QR 設定；已啟用 → 輸入驗證碼解鎖此 session 的後台
definePageMeta({ middleware: 'staff' })
const { t } = useI18n()
useHead({ title: () => t('admin.mfa.title') })
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
  <main class="c-page mf">
    <CreatorHeader :title="$t('admin.mfa.title')" back="/dashboard" />
    <section class="c-card mf-card">
      <template v-if="enabled">
        <p>{{ $t('admin.mfa.enabledPrompt') }}</p>
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
      <form v-if="enabled || setup" @submit.prevent="submit">
        <CreatorOtpInput ref="otp" v-model="code" @complete="submit" />
        <button class="c-btn primary block" :disabled="busy || code.length !== 6">{{ busy ? $t('admin.mfa.verifying') : enabled ? $t('admin.mfa.verify') : $t('admin.mfa.enable') }}</button>
      </form>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <p v-if="!enabled" class="c-mute"><NuxtLink to="/admin">{{ $t('admin.mfa.skip') }}</NuxtLink></p>
    </section>
  </main>
</template>
<style scoped>
.mfa-secret { font-family: var(--mono); word-break: break-all; text-align: center; padding: 10px; border: 1px dashed var(--ink-line); background: var(--bg-alt); letter-spacing: .12em; font-size: 13px; }
.mf-card{max-width:520px;margin:12px auto 0;padding:32px clamp(20px,5vw,40px);border:1px solid var(--ink-line);border-top:3px solid var(--red);box-shadow:8px 8px 0 var(--bg-alt),8px 8px 0 1px var(--line)}
.mf-card::before{content:"STAFF / 2FA";display:block;margin-bottom:16px;font:600 11px var(--mono);letter-spacing:.18em;color:var(--mute)}
.mf-card>p:first-of-type{font-size:17px;font-weight:700;line-height:1.6}
</style>
