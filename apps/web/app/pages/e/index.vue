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
    <span class="x-kicker" aria-hidden="true">RELIEF / EVENTS</span>
    <h1>{{ $t('guest.relief.heading') }}</h1>
    <div v-if="error" class="c-nf err"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <div v-else-if="!list.length" class="g-center g-mute">{{ $t('guest.relief.empty') }}</div>
    <div v-else class="g-grid">
      <article v-for="(e, n) in list" :key="e.slug" class="g-card">
        <span class="x-no" aria-hidden="true">No.{{ String(n + 1).padStart(2, '0') }}</span>
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
.x-kicker{display:block;margin:28px 0 0;font:700 11px var(--mono);letter-spacing:.16em;color:var(--mute)}
.x-kicker::before{content:"";display:inline-block;width:8px;height:8px;background:var(--red);margin-right:8px}
.x-kicker+h1{margin-top:8px}
.g-grid .g-card{border-top:2px solid var(--ink-line);padding:14px 16px 16px;display:flex;flex-direction:column;animation:rise .5s var(--ease) both}
.x-no{font:700 11px var(--mono);letter-spacing:.14em;color:var(--mute)}
.x-title{display:block;font-weight:800;font-size:19px;line-height:1.35;letter-spacing:-.01em;color:var(--fg);text-decoration:none;overflow-wrap:anywhere;padding:6px 0 4px;background:linear-gradient(var(--red),var(--red)) 0 100%/0 2px no-repeat;transition:background-size var(--t) var(--ease)}
.x-title:hover{background-size:100% 2px}
.x-crit{margin-left:0}
.x-head{display:flex;gap:16px;align-items:center;margin:8px 0 20px;padding-bottom:20px;border-bottom:1px solid var(--ink-line)}
.x-head h1{margin:0 0 6px}
.x-logo{flex:none;width:80px;height:80px;object-fit:contain;border:1px solid var(--line);padding:6px;background:var(--card)}
h2.x-h2{font:700 12px var(--mono);letter-spacing:.14em;text-transform:uppercase;margin:28px 0 14px;padding-top:12px;border-top:1px solid var(--ink-line)}
</style>
