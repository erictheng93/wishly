<script setup lang="ts">
import '~/assets/creator-pages.css'
import { safeRedirect } from '~/utils/redirect'
definePageMeta({ middleware: 'auth' })
const { t, locale } = useI18n()
useHead({ title: () => t('me.notifications.title') })
const { api } = useApi()
const { unread } = useInbox()
const items = ref<any[]>([]), cursor = ref<string | null>(null)
const loading = ref(true), more = ref(false), err = ref('')
async function load(next = false) {
  if (next) more.value = true; else loading.value = true
  err.value = ''
  try {
    const r = await api('/me/notifications', { query: { limit: 20, ...(next && cursor.value ? { before: cursor.value } : {}) } })
    items.value = next ? [...items.value, ...r.items] : r.items
    cursor.value = r.next_cursor; unread.value = r.unread
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false; more.value = false }
}
async function open(n: any) {
  if (!n.read_at) {
    try { await api(`/me/notifications/${n.id}/read`, { method: 'POST' }); n.read_at = new Date().toISOString(); unread.value = Math.max(0, unread.value - 1) } catch {}
  }
  const to = safeRedirect(n.link, '')
  if (to) await navigateTo(to)
}
async function readAll() {
  err.value = ''
  try { await api('/me/notifications/read-all', { method: 'POST' }); const now = new Date().toISOString(); items.value.forEach(n => { n.read_at ||= now }); unread.value = 0 } catch (e) { err.value = errMsg(e) }
}
const when = (iso: string) => new Date(iso).toLocaleString(locale.value === 'zh-Hant' ? 'zh-TW' : locale.value, { dateStyle: 'short', timeStyle: 'short' })
onMounted(() => load())
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('me.notifications.title')" back="/dashboard" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="loading" class="c-mute" role="status">{{ $t('me.loading') }}</p>
    <template v-else>
      <div class="c-row c-mb"><span class="c-grow c-mute">{{ $t('common.unreadCount', { n: unread }) }}</span><button class="c-btn" :disabled="!unread" @click="readAll">{{ $t('me.notifications.readAll') }}</button></div>
      <p v-if="!items.length" class="c-mute">{{ $t('me.notifications.empty') }}</p>
      <ul v-else class="c-inbox">
        <li v-for="n in items" :key="n.id" :class="{ unread: !n.read_at }">
          <button type="button" @click="open(n)"><span class="dot" aria-hidden="true" /><span class="t"><span v-if="!n.read_at" class="sr-only">{{ $t('me.notifications.unread') }}：</span>{{ n.title }}</span><time :datetime="n.created_at">{{ when(n.created_at) }}</time></button>
        </li>
      </ul>
      <button v-if="cursor" class="c-btn c-mt" :disabled="more" @click="load(true)">{{ $t('me.notifications.loadMore') }}</button>
    </template>
  </main>
</template>
