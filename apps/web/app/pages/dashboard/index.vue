<script setup lang="ts">
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('lists.dashboard.title') })
const { api } = useApi()
const list = ref<any[]>([])
const cursor = ref<string | null>(null)
const loading = ref(true), err = ref('')
const del = ref<any>(null), delBusy = ref(false), delErr = ref('')
async function load(more = false) {
  loading.value = true; err.value = ''
  try {
    const r = await api('/wishlists', { query: { limit: 20, cursor: more ? cursor.value : undefined } })
    list.value = more ? [...list.value, ...r.data] : r.data
    cursor.value = r.next_cursor
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
async function archive() {
  delBusy.value = true; delErr.value = ''
  try { await api(`/wishlists/${del.value.id}`, { method: 'DELETE' }); del.value = null; await load() }
  catch (e) { delErr.value = errMsg(e) } finally { delBusy.value = false }
}
onMounted(() => load())
const visible = computed(() => list.value.filter(w => w.status !== 'archived'))
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('lists.dashboard.title')" />
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">{{ $t('lists.common.retry') }}</button></p>
    <p v-else-if="loading && !list.length" class="c-mute" role="status">{{ $t('lists.common.loading') }}</p>
    <div v-else-if="!visible.length" class="c-center">
      <p class="c-mute">{{ $t('lists.dashboard.empty') }}</p>
      <NuxtLink to="/lists/new" class="c-btn primary">{{ $t('lists.dashboard.create') }}</NuxtLink>
    </div>
    <div class="c-grid">
    <article v-for="w in visible" :key="w.id" class="c-card" :class="{ bad: w.moderation_status === 'hidden' }">
      <div class="c-row">
        <img v-if="w.cover_image_url" class="c-thumb" :src="w.cover_image_url" alt="">
        <div class="c-grow">
          <NuxtLink :to="`/lists/${w.id}/edit`"><strong>{{ w.title }}</strong></NuxtLink>
          <div class="c-row wrap">
            <CreatorStatusBadge :v="w.moderation_status === 'hidden' ? 'hidden' : w.status" />
            <span class="c-mute">{{ $t('lists.common.personal') }}</span>
          </div>
          <div class="c-mute">{{ $t('lists.dashboard.completion', { pct: w.completion?.completion_pct ?? 0, done: w.completion?.fulfilled_count ?? 0, total: w.completion?.item_count ?? 0 }) }}</div>
        </div>
      </div>
      <div v-if="w.moderation_status === 'hidden'" class="c-err" role="alert">
        {{ $t('lists.dashboard.hidden', { reason: w.moderation_reason ? $t('lists.common.reasonSuffix', { reason: w.moderation_reason }) : '' }) }}
      </div>
      <div class="c-row c-mt">
        <NuxtLink :to="`/lists/${w.id}/edit`" class="c-btn c-grow">{{ $t('lists.common.edit') }}</NuxtLink>
        <NuxtLink :to="`/lists/${w.id}/progress`" class="c-btn c-grow">{{ $t('lists.common.progress') }}</NuxtLink>
        <button class="c-btn danger" @click="del = w; delErr = ''">{{ $t('lists.dashboard.archive') }}</button>
      </div>
    </article>
    </div>
    <button v-if="cursor" class="c-btn block" :disabled="loading" @click="load(true)">{{ $t('lists.dashboard.loadMore') }}</button>
    <NuxtLink v-if="visible.length" to="/lists/new" class="c-btn primary c-fab">{{ $t('lists.dashboard.newFab') }}</NuxtLink>
    <CreatorConfirm :open="!!del" :title="$t('lists.dashboard.archiveTitle')" :text="$t('lists.dashboard.archiveText', { title: del?.title })" :ok="$t('lists.dashboard.archiveOk')" danger :busy="delBusy" :error="delErr" @close="del = null" @ok="archive" />
  </main>
</template>
