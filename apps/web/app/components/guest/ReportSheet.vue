<script setup lang="ts">
import '~/assets/guest.css'
const props = defineProps<{ slug: string }>()
const emit = defineEmits<{ close: [], sent: [] }>()
const { api } = useGuest()
const { t, locale } = useI18n()
const reasons = ['scam', 'inappropriate', 'copyright', 'personal_info', 'other']
const reason = ref('scam'), detail = ref(''), busy = ref(false), msg = ref('')
// Turnstile：有設定 runtimeConfig.public.turnstileSiteKey 才載入 widget；否則送 'dev-bypass'（僅供本機 mock，後端需對應放行）
const siteKey = useRuntimeConfig().public.turnstileSiteKey as string
const token = ref(siteKey ? '' : 'dev-bypass')
const box = ref<HTMLElement>()
let wid: string | undefined
const SRC = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit'
function loadTurnstile(): Promise<any> {
  const w = window as any
  if (w.turnstile) return Promise.resolve(w.turnstile)
  return new Promise((ok, no) => {
    const s = document.querySelector<HTMLScriptElement>(`script[src="${SRC}"]`) ?? Object.assign(document.createElement('script'), { src: SRC, async: true })
    s.addEventListener('load', () => ok(w.turnstile)); s.addEventListener('error', () => { s.remove(); no(new Error('load')) })
    if (!s.isConnected) document.head.append(s)
  })
}
function reset() { token.value = ''; if (wid !== undefined) (window as any).turnstile?.reset(wid) }
onMounted(async () => {
  if (!siteKey) return
  try {
    const ts = await loadTurnstile()
    if (!box.value) return // 載入期間 sheet 已關閉
    wid = ts.render(box.value, {
      sitekey: siteKey, theme: (document.documentElement.dataset.theme ?? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')), language: ({ 'zh-Hant': 'zh-tw', en: 'en', vi: 'vi' } as Record<string, string>)[String(locale.value)] ?? 'auto',
      callback: (v: string) => { token.value = v; msg.value = '' },
      'expired-callback': () => { token.value = ''; msg.value = t('guest.turnstile.expired') },
      'timeout-callback': () => { token.value = ''; msg.value = t('guest.turnstile.timeout') },
      'error-callback': () => { token.value = ''; msg.value = t('guest.turnstile.error') },
    })
  } catch { msg.value = t('guest.turnstile.loadFailReopen') }
})
onBeforeUnmount(() => { if (wid !== undefined) (window as any).turnstile?.remove(wid); wid = undefined })
async function submit() {
  if (busy.value) return
  if (!token.value) { msg.value = t('guest.turnstile.required'); return }
  busy.value = true; msg.value = ''
  try {
    await api(`/public/wishlists/${props.slug}/reports`, { method: 'POST', body: { reason: reason.value, detail: detail.value || null, item_id: null, turnstile_token: token.value } })
    emit('sent')
  } catch (e: any) { const r = reportErrMsg(e); msg.value = r.msg; if (r.resetCaptcha && siteKey) reset() } finally { busy.value = false }
}
</script>
<template>
  <GuestDialog labelledby="report-title" :dirty="!!detail" @close="emit('close')">
    <form class="g-sheet" @submit.prevent="submit">
      <div id="report-title" class="g-title">{{ $t('guest.report.title') }}</div>
      <fieldset class="g-fieldset"><legend class="g-mute">{{ $t('guest.report.reasonLegend') }}</legend>
        <label v-for="v in reasons" :key="v" class="g-radio">
          <input v-model="reason" type="radio" :value="v"> {{ $t('guest.report.reason.' + v) }}</label>
      </fieldset>
      <label for="rd">{{ $t('guest.report.detailLabel') }}</label>
      <textarea id="rd" v-model="detail" rows="3" maxlength="500" :placeholder="$t('guest.report.placeholder')" />
      <div v-if="siteKey" ref="box" style="margin:12px 0;min-height:65px" />
      <div v-if="msg" class="g-banner err" role="alert">{{ msg }}</div>
      <button class="g-btn" :disabled="busy || !token">{{ busy ? $t('guest.sending') : $t('guest.report.send') }}</button>
      <p class="g-mute">{{ $t('guest.report.note') }}</p>
      <button type="button" class="g-link" @click="emit('close')">{{ $t('guest.cancel') }}</button>
    </form>
  </GuestDialog>
</template>
