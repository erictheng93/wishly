<script setup lang="ts">
// Cloudflare Turnstile（explicit render）；v-model 為 token。未設 siteKey（本機）不顯示；後端無 secret 時本來就不會要求驗證。
const token = defineModel<string>({ default: '' })
const { t, locale } = useI18n()
const err = ref('')
const siteKey = useRuntimeConfig().public.turnstileSiteKey as string
const box = ref<HTMLElement>()
let wid: string | undefined
const SRC = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit'
function load(): Promise<any> {
  const w = window as any
  if (w.turnstile) return Promise.resolve(w.turnstile)
  return new Promise((ok, no) => {
    const s = document.querySelector<HTMLScriptElement>(`script[src="${SRC}"]`) ?? Object.assign(document.createElement('script'), { src: SRC, async: true })
    s.addEventListener('load', () => ok(w.turnstile)); s.addEventListener('error', () => { s.remove(); no(new Error('load')) })
    if (!s.isConnected) document.head.append(s)
  })
}
onMounted(async () => {
  if (!siteKey) return
  try {
    const ts = await load()
    if (!box.value) return
    wid = ts.render(box.value, {
      sitekey: siteKey, theme: (document.documentElement.dataset.theme ?? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')), language: ({ 'zh-Hant': 'zh-tw', en: 'en', vi: 'vi', id: 'id' } as Record<string, string>)[String(locale.value)] ?? 'auto',
      callback: (v: string) => { token.value = v; err.value = '' },
      'expired-callback': () => { token.value = ''; err.value = t('guest.turnstile.expired') },
      'timeout-callback': () => { token.value = ''; err.value = t('guest.turnstile.timeout') },
      'error-callback': () => { token.value = ''; err.value = t('guest.turnstile.error') },
    })
  } catch { err.value = t('guest.turnstile.loadFailRetry') }
})
onBeforeUnmount(() => { if (wid !== undefined) (window as any).turnstile?.remove(wid); wid = undefined })
</script>
<template>
  <div v-if="siteKey" style="margin:12px 0;min-height:65px">
    <div ref="box" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
  </div>
</template>
