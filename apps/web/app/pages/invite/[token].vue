<script setup lang="ts">
const { t } = useI18n()
useHead({ title: () => t('auth.invite.title'), meta: [{ name: 'robots', content: 'noindex' }] })
const route = useRoute()
const token = String(route.params.token)
const { api } = useApi()
const { user, fetchMe } = useAuth()
const inviter = ref<any>(null), list = ref<any>(null), state = ref<'loading' | 'ok' | 'gone' | 'error'>('loading')
const busy = ref(false), err = ref('')
onMounted(async () => {
  await fetchMe()
  try { const r = await api(`/friends/invites/${encodeURIComponent(token)}`, { noRedirect: true }); inviter.value = r.inviter; list.value = r.wishlist; state.value = 'ok' }
  catch (e: any) { state.value = e.status === 404 ? 'gone' : 'error' }
})
async function accept() {
  busy.value = true; err.value = ''
  try { const r = await api(`/friends/invites/${encodeURIComponent(token)}/accept`, { method: 'POST' }); await navigateTo(r?.wishlist ? `/s/${r.wishlist.slug}` : '/friends') }
  catch (e: any) { err.value = e.status === 404 ? t('auth.invite.expired') : apiErrMsg(e) } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('auth.invite.title')" back="/dashboard" />
    <p v-if="state === 'loading'" class="c-mute" role="status">{{ $t('auth.invite.loading') }}</p>
    <section v-else-if="state === 'gone'" class="c-gone"><span class="pf-kick" aria-hidden="true">INVITE / 404</span><p class="c-gone-t">{{ $t('auth.invite.gone') }}</p><p class="c-mute">{{ $t('auth.invite.goneHint') }}</p></section>
    <section v-else-if="state === 'error'" class="c-gone err"><span class="pf-kick" aria-hidden="true">INVITE / ERR</span><p class="c-err c-gone-t" role="alert">{{ $t('auth.invite.loadFail') }}</p></section>
    <section v-else class="pf-hero">
      <span class="pf-kick" aria-hidden="true">INVITE / FRIEND</span>
      <h2 class="c-h2">{{ list ? $t('auth.invite.invitesList', { name: inviter.display_name, title: list.title }) : $t('auth.invite.invites', { name: inviter.display_name }) }}</h2>
      <p v-if="inviter.handle" class="c-mute">@{{ inviter.handle }}</p>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <button v-if="user" class="c-btn primary" :disabled="busy" @click="accept">{{ busy ? $t('auth.invite.busy') : $t('auth.invite.accept') }}</button>
      <NuxtLink v-else :to="{ path: '/login', query: { redirect: route.fullPath } }" class="c-btn primary">{{ $t('auth.invite.loginToAccept') }}</NuxtLink>
    </section>
  </main>
</template>

<style scoped>
.pf-kick{display:block;margin:0 0 8px;font:700 11px var(--mono);letter-spacing:.16em;color:var(--mute)}
.pf-kick::before{content:"";display:inline-block;width:8px;height:8px;background:var(--red);margin-right:8px}
.pf-hero{border:1px solid var(--ink-line);border-top:3px solid var(--red);padding:24px 20px;margin:16px 0 8px;display:grid;gap:10px;justify-items:start;background:var(--card)}
.pf-hero .c-h2{font-size:clamp(28px,6vw,40px);font-weight:900;letter-spacing:-.02em;margin:0;line-height:1.1}
.pf-hero p{margin:0}
.pf-hero .c-h2::before{display:none}
.pf-av{width:64px;height:64px;display:grid;place-items:center;background:var(--fg);color:var(--bg);font:900 28px var(--font)}
.pf-sec{font:700 12px var(--mono);letter-spacing:.14em;text-transform:uppercase;margin:32px 0 12px;padding-top:12px;border-top:1px solid var(--ink-line)}
.pf-sec span{color:var(--red-text);margin-right:8px}
.pf-card{border-top:2px solid var(--ink-line)!important;transition:transform var(--t) var(--ease),box-shadow var(--t) var(--ease)}
@media (hover:hover){.pf-card:hover{transform:translate(-2px,-2px);box-shadow:6px 6px 0 var(--bg-alt),6px 6px 0 1px var(--line)}}
</style>
