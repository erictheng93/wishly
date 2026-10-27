<script setup lang="ts">
// 災害救援事件列表（訪客可看，SSR；API 回應由 CDN 快取，不帶 cookie）
const { public: { apiBase } } = useRuntimeConfig()
const { t } = useI18n()
useSeoMeta({ title: () => t('guest.relief.seoTitle'), description: () => t('guest.relief.seoDesc') })
const { data, error, refresh } = await useAsyncData('relief-events', () => $fetch<any>(`${apiBase}/api/v1/public/relief-events`))
const extra = ref<any[]>([]), extraNext = ref<number | null | undefined>(undefined), busy = ref(false), moreErr = ref(false)
watch(data, () => { extra.value = []; extraNext.value = undefined })
const list = computed(() => [...(data.value?.events ?? []), ...extra.value])
const next = computed(() => extraNext.value !== undefined ? extraNext.value : data.value?.next_page ?? null)
async function more() {
  busy.value = true; moreErr.value = false
  try {
    const r = await $fetch<any>(`${apiBase}/api/v1/public/relief-events`, { query: { page: next.value } })
    extra.value = [...extra.value, ...r.events]; extraNext.value = r.next_page
  } catch { moreErr.value = true } finally { busy.value = false }
}
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <h1>{{ $t('guest.relief.heading') }}</h1>
    <div v-if="error" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <div v-else-if="!list.length" class="g-center g-mute">{{ $t('guest.relief.empty') }}</div>
    <div v-else class="g-grid">
      <article v-for="e in list" :key="e.slug" class="g-card">
        <NuxtLink :to="`/e/${e.slug}`" class="x-title">{{ e.name }}</NuxtLink>
        <p class="g-mute">
          {{ $t('org.relief.hazard.' + e.hazard_type) }}・{{ e.counties.join('、') }}<template v-if="e.occurred_on">・{{ e.occurred_on }}</template>
        </p>
      </article>
    </div>
    <div v-if="next" class="g-center">
      <p v-if="moreErr" class="g-field-err" role="alert">{{ $t('guest.explore.moreErr') }}</p>
      <button class="g-btn ghost" :disabled="busy" @click="more">{{ busy ? $t('guest.loading') : $t('guest.relief.loadMore') }}</button>
    </div>
  </main>
</template>

<style scoped>
.x-title{display:block;font-weight:700;font-size:18px;color:var(--fg);text-decoration:none;overflow-wrap:anywhere;padding:8px 0}
</style>
