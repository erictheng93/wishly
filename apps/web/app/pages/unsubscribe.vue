<script setup lang="ts">
import '~/assets/guest.css'
// 退訂捐助通知（二步）：信件連結 → GET /unsubscribe 302 到這裡；必須由人按鈕才 POST，避免信件掃描器預取就誤退訂（F-26）。
// 舊流程 /unsubscribed?ok=1 仍保留（pages/unsubscribed.vue）。
const { t } = useI18n()
useSeoMeta({ title: () => t('guest.unsub.seoTitle'), robots: 'noindex' })
useHead({ meta: [{ name: 'referrer', content: 'no-referrer' }] }) // token 在網址上，不外洩
const token = String(useRoute().query.token ?? '')
const { api } = useGuest()
// 權杖前段是 base64 的 `kind:id:exp`（非機密，簽章在後段）；生日通知（b）與送禮通知用不同文案
const birthday = (() => { try { return atob(token.split('.')[0]!.replace(/-/g, '+').replace(/_/g, '/')).startsWith('b:') } catch { return false } })()
const k = (key: string) => `guest.unsub.${key}${birthday ? 'Birthday' : ''}`
const state = ref<'ask' | 'busy' | 'done' | 'invalid' | 'error'>(token ? 'ask' : 'invalid')
const msg = ref('')
async function submit() {
  state.value = 'busy'
  try { await api('/unsubscribe', { method: 'POST', body: { token } }); state.value = 'done' }
  catch (e: any) {
    if (e.code === 'INVALID_TOKEN' || e.status === 422) state.value = 'invalid'
    else { msg.value = e.detail; state.value = 'error' }
  }
}
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <div class="x-n">
    <div class="x-n-head" aria-hidden="true"><span>NOTICE / UNSUBSCRIBE</span><span>No.0090</span></div>
    <div class="x-n-body">
    <template v-if="state === 'done'">
      <h1>{{ $t('guest.unsub.doneTitle') }}</h1>
      <p class="g-mute">{{ $t(k('doneBody')) }}</p>
      <NuxtLink to="/settings">{{ $t('guest.unsub.toSettings') }}</NuxtLink>
    </template>
    <template v-else-if="state === 'invalid'">
      <h1>{{ $t('guest.unsub.invalidTitle') }}</h1>
      <p class="g-mute">{{ $t('guest.unsub.invalidBody') }}</p>
      <NuxtLink to="/settings">{{ $t('guest.unsub.toSettings') }}</NuxtLink>
    </template>
    <template v-else>
      <h1>{{ $t(k('askTitle')) }}</h1>
      <p class="g-mute">{{ $t(k('askBody')) }}</p>
      <div v-if="state === 'error'" class="g-banner err" role="alert">{{ msg }}</div>
      <button class="g-btn" :disabled="state === 'busy'" @click="submit">{{ state === 'busy' ? $t('guest.unsub.processing') : $t('guest.unsub.confirm') }}</button>
    </template>
    </div>
    </div>
  </main>
</template>
<style scoped>
.x-n{max-width:560px;margin:56px auto 80px;padding:0 0 28px;border:1px solid var(--ink-line);background:var(--card);box-shadow:10px 10px 0 var(--bg-alt),10px 10px 0 1px var(--line);text-align:left}
.x-n-head{display:flex;justify-content:space-between;padding:12px 20px;background:var(--hatch);border-bottom:1px solid var(--ink-line);font:700 11px/1.4 var(--mono);letter-spacing:.16em;color:var(--mute)}
.x-n-head span:last-child{color:var(--red-text)}
.x-n-body{padding:28px 24px 0}
.x-n h1{margin:0 0 12px;font-size:clamp(28px,6vw,40px);font-weight:900;letter-spacing:-.03em}
.x-n p{margin:0 0 20px}
.x-n .g-btn{width:100%}
.x-n a{display:inline-flex;align-items:center;min-height:44px;font-weight:700;text-decoration:underline;text-decoration-color:var(--red);text-underline-offset:5px}
@media (max-width:520px){.x-n{margin:32px 0 56px;box-shadow:none}}
</style>
