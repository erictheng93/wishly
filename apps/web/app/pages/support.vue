<script setup lang="ts">
// 客服表單（#92）：已登入送到 /me/support-requests 並列出自己的請求；未登入（含被停權而無法登入者）
// 填 Email + Turnstile 送到 /public/support-requests，營運以 Email 回覆、站內無查詢。
// 登入判斷：只有 ws_hint 在時才抓 /me（noRedirect），不會被導去登入頁
import '~/assets/creator-pages.css'
const { t } = useI18n()
useSeoMeta({ title: () => t('me.support.title'), robots: 'noindex' })
const { api } = useApi()
const { user, fetchMe } = useAuth()
const route = useRoute()
const CATEGORIES = ['account', 'wishlist', 'payment', 'wallet', 'other'] as const
const q = (k: string) => (typeof route.query[k] === 'string' ? route.query[k] as string : '')

const ready = ref(false), loading = ref(false), listErr = ref('')
const rows = ref<any[]>([])
const category = ref<string>((CATEGORIES as readonly string[]).includes(q('category')) ? q('category') : 'other')
const wishlistId = ref(/^[0-9a-f-]{36}$/i.test(q('wishlist')) ? q('wishlist') : '')
const body = ref(''), email = ref(''), tsToken = ref(''), capKey = ref(0)
const busy = ref(false), err = ref(''), sent = ref('')

async function load() {
  loading.value = true; listErr.value = ''
  try { rows.value = (await api('/me/support-requests')).data ?? [] } catch (e) { listErr.value = errMsg(e) } finally { loading.value = false }
}
onMounted(async () => {
  if (!user.value && useAuthHint().value) await fetchMe()
  ready.value = true
  if (user.value) await load()
})

async function submit() {
  if (busy.value) return
  err.value = ''; sent.value = ''
  if (!body.value.trim()) { err.value = t('me.support.bodyRequired'); return }
  if (!user.value && !email.value.trim()) { err.value = t('me.support.emailRequired'); return }
  busy.value = true
  try {
    if (user.value) {
      const r = await api('/me/support-requests', { method: 'POST', body: { category: category.value, body: body.value, ...(wishlistId.value && category.value === 'wishlist' ? { wishlist_id: wishlistId.value } : {}) } })
      rows.value = [r, ...rows.value]
      sent.value = t('me.support.sent')
    } else {
      await api('/public/support-requests', { method: 'POST', noRedirect: true, body: { email: email.value.trim(), category: category.value, body: body.value, turnstile_token: tsToken.value || undefined } })
      sent.value = t('me.support.sentGuest')
    }
    body.value = ''; wishlistId.value = ''
  } catch (e: any) {
    err.value = errMsg(e)
    if (!user.value) { tsToken.value = ''; capKey.value++ } // Turnstile token 只能用一次
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('me.support.title')" />
    <p class="c-mute">{{ $t('me.support.intro') }}</p>
    <p v-if="!ready" class="c-mute" role="status">{{ $t('me.loading') }}</p>
    <template v-else>
      <div aria-live="polite"><p v-if="sent" class="c-ok" role="status" data-testid="sent">{{ sent }}</p></div>
      <form class="c-card sp-form" novalidate data-testid="support-form" @submit.prevent="submit">
        <label v-if="!user" class="c-field"><span>{{ $t('me.support.email') }}</span><input v-model="email" type="email" autocomplete="email" maxlength="254" placeholder="name@example.com" required data-testid="email"></label>
        <p v-if="!user" class="c-mute c-small">{{ $t('me.support.emailHint') }}</p>
        <label class="c-field"><span>{{ $t('me.support.category') }}</span>
          <select v-model="category" data-testid="category"><option v-for="c in CATEGORIES" :key="c" :value="c">{{ $t(`me.support.categories.${c}`) }}</option></select>
        </label>
        <p v-if="user && wishlistId && category === 'wishlist'" class="c-mute c-small" data-testid="wishlist-ref">{{ $t('me.support.wishlistRef') }}</p>
        <label class="c-field"><span>{{ $t('me.support.body') }}</span><textarea v-model="body" rows="6" maxlength="2000" :placeholder="$t('me.support.placeholder')" required data-testid="body" /></label>
        <Turnstile v-if="!user" :key="capKey" v-model="tsToken" />
        <p v-if="err" class="c-err" role="alert" data-testid="err">{{ err }}</p>
        <button class="c-btn primary block" type="submit" :disabled="busy">{{ busy ? $t('me.support.submitting') : $t('me.support.submit') }}</button>
      </form>
      <p class="c-mute c-small">{{ $t('me.support.emailAlt') }}<SupportLink :form="false" /></p>

      <template v-if="user">
        <h2 class="c-h2 x-h">{{ $t('me.support.mine') }}</h2>
        <p v-if="loading" class="c-mute" role="status">{{ $t('me.loading') }}</p>
        <p v-if="listErr" class="c-err" role="alert">{{ listErr }} <button class="c-btn" @click="load">{{ $t('me.retry') }}</button></p>
        <p v-if="!loading && !rows.length && !listErr" class="x-empty" data-testid="empty">{{ $t('me.support.empty') }}</p>
        <article v-for="r in rows" :key="r.id" class="c-card" :data-status="r.status">
          <div class="c-row wrap">
            <strong class="c-grow">{{ $t(`me.support.categories.${r.category}`) }}</strong>
            <span class="c-badge">{{ $t(`me.support.status.${r.status}`) }}</span>
          </div>
          <div class="c-mute c-small">{{ $t('me.support.createdAt', { time: fmtTime(r.created_at) }) }}</div>
          <p class="sp-text">{{ r.body }}</p>
          <div v-if="r.reply" class="sp-reply">
            <div class="c-lbl">{{ $t('me.support.reply') }}<span v-if="r.handled_at" class="c-mute c-small">・{{ fmtTime(r.handled_at) }}</span></div>
            <p class="sp-text c-m0">{{ r.reply }}</p>
          </div>
        </article>
      </template>
    </template>
  </main>
</template>
<style scoped>
.sp-form{margin:16px 0;padding:20px}
.sp-text{white-space:pre-wrap;overflow-wrap:anywhere}
.sp-reply{margin-top:12px;padding-top:12px;border-top:1px solid var(--line)}
</style>
