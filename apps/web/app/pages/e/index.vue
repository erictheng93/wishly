<script setup lang="ts">
// 災害救援事件列表（訪客可看，SSR；API 回應由 CDN 快取，不帶 cookie）
const { public: { apiBase } } = useRuntimeConfig()
const { t } = useI18n()
useSeoMeta({ title: () => t('guest.relief.seoTitle'), description: () => t('guest.relief.seoDesc') })
// 功能關閉時回真 404（同 /org），否則爬蟲會收錄一頁空殼
if (!(await useOrgsEnabled()).value) throw createError({ statusCode: 404, fatal: true })
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
    <div v-if="error" class="c-nf err"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <div v-else-if="!list.length" class="g-center g-mute">{{ $t('guest.relief.empty') }}</div>
    <div v-else class="g-grid">
      <article v-for="e in list" :key="e.slug" class="g-card">
        <NuxtLink :to="localizedPath(`/e/${e.slug}`, $i18n.locale)" class="x-title">{{ e.name }}</NuxtLink>
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
.g-wrap h1{position:relative;margin:44px 0 16px;padding-bottom:18px;border-bottom:1px solid var(--line);font-weight:900;letter-spacing:-.03em}
.g-wrap h1::after{content:"";position:absolute;left:0;bottom:-1px;width:48px;height:2px;background:var(--red)}
html:lang(vi) .g-wrap h1{line-height:1.22}
.g-grid .g-card{padding:14px 16px 16px;display:flex;flex-direction:column;animation:rise .5s var(--ease) both}
.x-title{display:block;font-weight:800;font-size:19px;line-height:1.35;letter-spacing:-.01em;color:var(--fg);text-decoration:none;overflow-wrap:anywhere;padding:6px 0 4px;background:linear-gradient(var(--red),var(--red)) 0 100%/0 2px no-repeat;transition:background-size var(--t) var(--ease)}
.x-title:hover{background-size:100% 2px}
/* 空狀態：虛線斜線底的標本盒，頂端一枚線條圖釘 */
.g-center.g-mute{display:grid;justify-items:center;gap:14px;margin:28px 0 0;padding:52px 20px;border:1px dashed var(--ink-line);background:var(--hatch),var(--card);font-size:14px;color:var(--mute);text-align:center}
.g-center.g-mute::before{content:"";width:34px;height:34px;background:var(--fg);-webkit-mask:var(--pin) center/contain no-repeat;mask:var(--pin) center/contain no-repeat;--pin:url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%23000' stroke-width='1.2' stroke-linejoin='round'%3E%3Cpath d='M12 21s-7-6.1-7-11.5a7 7 0 0 1 14 0C19 14.900 12 21 12 21z'/%3E%3Ccircle cx='12' cy='9.500' r='2.500'/%3E%3C/svg%3E")}
</style>
