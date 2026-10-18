<script setup lang="ts">
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('common.friends') })
const { api } = useApi()
const data = ref<any>({ friends: [], incoming: [], outgoing: [] })
const loading = ref(true), err = ref(''), msg = ref('')
const to = ref(''), sending = ref(false)
const invite = ref<any>(null), copied = ref(false)
const unfriend = ref<any>(null), ufBusy = ref(false), ufErr = ref('')

const msgOf = (e: any) => apiErrMsg(e)
async function load() {
  try { data.value = await api('/friends') } catch (e) { err.value = msgOf(e) } finally { loading.value = false }
}
onMounted(load)
// 動作後重新載入清單；失敗顯示錯誤
async function act(f: () => Promise<any>) {
  err.value = msg.value = ''
  try { await f(); await load() } catch (e) { err.value = msgOf(e) }
}
async function send() {
  const v = to.value.trim().replace(/^@/, '')
  if (!v) return
  sending.value = true; err.value = msg.value = ''
  try {
    await api('/friends/requests', { method: 'POST', body: { to: v } })
    msg.value = t('me.friends.sent'); to.value = ''; await load()
  } catch (e) { err.value = msgOf(e) } finally { sending.value = false }
}
const accept = (r: any) => act(() => api(`/friends/requests/${r.id}/accept`, { method: 'POST' }))
const dropReq = (r: any) => act(() => api(`/friends/requests/${r.id}`, { method: 'DELETE' }))
async function doUnfriend() {
  ufBusy.value = true; ufErr.value = ''
  try { await api(`/friends/${unfriend.value.id}`, { method: 'DELETE' }); unfriend.value = null; await load() }
  catch (e) { ufErr.value = msgOf(e) } finally { ufBusy.value = false }
}
const makeInvite = () => act(async () => { invite.value = await api('/friends/invites', { method: 'POST' }); copied.value = false })
const revoke = () => act(async () => { await api('/friends/invites', { method: 'DELETE' }); invite.value = null })
async function copy() {
  try { await copyText(invite.value.url); copied.value = true } catch { err.value = t('me.friends.copyFail') }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('common.friends')" back="/dashboard" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="loading" class="c-mute" role="status">{{ $t('me.loading') }}</p>
    <template v-else>
      <section class="c-card">
        <h2 class="c-h2">{{ $t('me.friends.addTitle') }}</h2>
        <form @submit.prevent="send">
          <label class="c-field"><span>{{ $t('me.friends.toLabel') }}</span><input v-model="to" maxlength="120" required :placeholder="$t('me.friends.toPh')"></label>
          <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
          <button class="c-btn primary" :disabled="sending">{{ sending ? $t('me.friends.sending') : $t('me.friends.send') }}</button>
        </form>
      </section>

      <section class="c-card">
        <h2 class="c-h2">{{ $t('me.friends.inviteTitle') }}</h2>
        <p class="c-mute">{{ $t('me.friends.inviteHint') }}</p>
        <template v-if="invite">
          <label class="c-field"><span>{{ $t('me.friends.inviteUrl') }}</span><input :value="invite.url" readonly @focus="($event.target as HTMLInputElement).select()"></label>
          <CreatorQr :text="invite.url" />
          <div class="c-row wrap c-mt">
            <button class="c-btn" @click="copy">{{ copied ? $t('me.friends.copied') : $t('me.friends.copy') }}</button>
            <button class="c-btn" @click="makeInvite">{{ $t('me.friends.regen') }}</button>
            <button class="c-btn danger" @click="revoke">{{ $t('me.friends.revoke') }}</button>
          </div>
        </template>
        <template v-else>
          <button class="c-btn primary" @click="makeInvite">{{ $t('me.friends.gen') }}</button>
          <button class="c-btn danger" @click="revoke">{{ $t('me.friends.revokeExisting') }}</button>
        </template>
      </section>

      <section v-if="data.incoming.length" class="c-card">
        <h2 class="c-h2">{{ $t('me.friends.incoming') }}</h2>
        <div v-for="r in data.incoming" :key="r.id" class="c-row wrap c-mb">
          <span class="c-grow"><strong>{{ r.user.display_name }}</strong><small v-if="r.user.handle" class="c-mute"> @{{ r.user.handle }}</small></span>
          <button class="c-btn primary" @click="accept(r)">{{ $t('me.friends.accept') }}</button>
          <button class="c-btn" @click="dropReq(r)">{{ $t('me.friends.decline') }}</button>
        </div>
      </section>

      <section v-if="data.outgoing.length" class="c-card">
        <h2 class="c-h2">{{ $t('me.friends.outgoing') }}</h2>
        <div v-for="r in data.outgoing" :key="r.id" class="c-row wrap c-mb">
          <span class="c-grow"><strong>{{ r.user.display_name }}</strong><small v-if="r.user.handle" class="c-mute"> @{{ r.user.handle }}</small></span>
          <button class="c-btn" @click="dropReq(r)">{{ $t('me.friends.cancelInvite') }}</button>
        </div>
      </section>

      <section class="c-card">
        <h2 class="c-h2">{{ $t('me.friends.mine', { n: data.friends.length }) }}</h2>
        <p v-if="!data.friends.length" class="c-mute">{{ $t('me.friends.none') }}</p>
        <div v-for="f in data.friends" :key="f.id" class="c-row wrap c-mb">
          <span class="c-grow">
            <NuxtLink v-if="f.handle" :to="`/u/${f.handle}`"><strong>{{ f.display_name }}</strong></NuxtLink>
            <strong v-else>{{ f.display_name }}</strong>
            <small v-if="f.handle" class="c-mute"> @{{ f.handle }}</small>
          </span>
          <button class="c-btn" @click="unfriend = f; ufErr = ''">{{ $t('me.friends.unfriend') }}</button>
        </div>
      </section>
    </template>
    <CreatorConfirm :open="!!unfriend" :title="$t('me.friends.unfriendTitle', { name: unfriend?.display_name })" :text="$t('me.friends.unfriendText')" :ok="$t('me.friends.unfriend')" danger :busy="ufBusy" :error="ufErr" @close="unfriend = null" @ok="doUnfriend" />
  </main>
</template>
