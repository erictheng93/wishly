<script setup lang="ts">
import '~/assets/guest.css'
// 退訂捐助通知（二步）：信件連結 → GET /unsubscribe 302 到這裡；必須由人按鈕才 POST，避免信件掃描器預取就誤退訂（F-26）。
// 舊流程 /unsubscribed?ok=1 仍保留（pages/unsubscribed.vue）。
const { t } = useI18n()
useSeoMeta({ title: () => t('guest.unsub.seoTitle'), robots: 'noindex' })
useHead({ meta: [{ name: 'referrer', content: 'no-referrer' }] }) // token 在網址上，不外洩
const token = String(useRoute().query.token ?? '')
const { api } = useGuest()
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
  <main class="g-wrap g-center" style="padding-inline:16px">
    <template v-if="state === 'done'">
      <h1>{{ $t('guest.unsub.doneTitle') }}</h1>
      <p class="g-mute">{{ $t('guest.unsub.doneBody') }}</p>
      <NuxtLink to="/settings">{{ $t('guest.unsub.toSettings') }}</NuxtLink>
    </template>
    <template v-else-if="state === 'invalid'">
      <h1>{{ $t('guest.unsub.invalidTitle') }}</h1>
      <p class="g-mute">{{ $t('guest.unsub.invalidBody') }}</p>
      <NuxtLink to="/settings">{{ $t('guest.unsub.toSettings') }}</NuxtLink>
    </template>
    <template v-else>
      <h1>{{ $t('guest.unsub.askTitle') }}</h1>
      <p class="g-mute">{{ $t('guest.unsub.askBody') }}</p>
      <div v-if="state === 'error'" class="g-banner err" role="alert">{{ msg }}</div>
      <button class="g-btn" :disabled="state === 'busy'" @click="submit">{{ state === 'busy' ? $t('guest.unsub.processing') : $t('guest.unsub.confirm') }}</button>
    </template>
  </main>
</template>
