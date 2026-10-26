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
    <div v-else-if="state === 'gone'" class="c-center"><p>{{ $t('auth.invite.gone') }}</p><p class="c-mute">{{ $t('auth.invite.goneHint') }}</p></div>
    <div v-else-if="state === 'error'" class="c-center"><p class="c-err" role="alert">{{ $t('auth.invite.loadFail') }}</p></div>
    <section v-else class="c-card">
      <h2 class="c-h2">{{ list ? $t('auth.invite.invitesList', { name: inviter.display_name, title: list.title }) : $t('auth.invite.invites', { name: inviter.display_name }) }}</h2>
      <p v-if="inviter.handle" class="c-mute">@{{ inviter.handle }}</p>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <button v-if="user" class="c-btn primary" :disabled="busy" @click="accept">{{ busy ? $t('auth.invite.busy') : $t('auth.invite.accept') }}</button>
      <NuxtLink v-else :to="{ path: '/login', query: { redirect: route.fullPath } }" class="c-btn primary">{{ $t('auth.invite.loginToAccept') }}</NuxtLink>
    </section>
  </main>
</template>
