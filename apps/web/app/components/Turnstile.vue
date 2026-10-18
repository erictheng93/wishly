<script setup lang="ts">
// Cloudflare Turnstile（explicit render）；v-model 為 token。未設 siteKey（本機）不顯示；後端無 secret 時本來就不會要求驗證。
const token = defineModel<string>({ default: '' })
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
    const t = await load()
    if (!box.value) return
    wid = t.render(box.value, {
      sitekey: siteKey, theme: (document.documentElement.dataset.theme ?? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')), language: 'zh-tw',
      callback: (v: string) => { token.value = v; err.value = '' },
      'expired-callback': () => { token.value = ''; err.value = '驗證已過期，請重新驗證' },
      'timeout-callback': () => { token.value = ''; err.value = '驗證逾時，請重新驗證' },
      'error-callback': () => { token.value = ''; err.value = '人機驗證發生錯誤，請重新驗證或稍後再試' },
    })
  } catch { err.value = '無法載入人機驗證，請檢查網路後重試' }
})
onBeforeUnmount(() => { if (wid !== undefined) (window as any).turnstile?.remove(wid); wid = undefined })
</script>
<template>
  <div v-if="siteKey" style="margin:12px 0;min-height:65px">
    <div ref="box" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
  </div>
</template>
