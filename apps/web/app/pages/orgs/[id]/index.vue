<script setup lang="ts">
// 組織後台總覽：事件與據點；owner / coordinator 可新增，volunteer 唯讀
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('org.console.title') })
const { api } = useApi()
const id = useRoute().params.id as string
const { orgs, current, adopt } = useOrgMode()
const role = computed(() => orgs.value.find(o => o.id === id)?.role)
const canEdit = computed(() => role.value === 'owner' || role.value === 'coordinator')
const org = ref<any>(null), sites = ref<any[]>([]), events = ref<any[]>([])
const loading = ref(true), err = ref('')
if (current.value !== id) adopt(id)
onMounted(async () => {
  try {
    ;[org.value, sites.value, events.value] = await Promise.all([api(`/orgs/${id}`), api(`/orgs/${id}/sites`), api(`/orgs/${id}/events`)])
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
})
// 契約：據點是 wishlist JSON，可能包在 wishlist 欄位內
const site = (s: any) => s.wishlist ?? s
</script>

<template>
  <main class="c-page">
    <CreatorHeader :title="org?.display_name || $t('org.console.title')" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="loading" class="c-mute" role="status">{{ $t('org.apply.loading') }}</p>
    <template v-else-if="org">
      <nav class="c-row">
        <NuxtLink v-if="role === 'owner'" :to="`/orgs/${id}/members`" class="c-btn">{{ $t('org.console.members') }}</NuxtLink>
      </nav>

      <section class="c-card">
        <h2 class="c-h2">{{ $t('org.console.events') }}</h2>
        <NuxtLink v-if="canEdit" :to="`/orgs/${id}/events/new`" class="c-btn primary">{{ $t('org.console.newEvent') }}</NuxtLink>
        <p v-if="!events.length" class="c-mute">{{ $t('org.console.noEvents') }}</p>
        <ul class="c-list">
          <li v-for="e in events" :key="e.id">
            <strong>{{ e.name }}</strong>
            <span class="c-mute">{{ e.occurred_on }}　{{ (e.counties ?? []).join('、') }}</span>
          </li>
        </ul>
      </section>

      <section class="c-card">
        <h2 class="c-h2">{{ $t('org.console.sites') }}</h2>
        <NuxtLink v-if="canEdit" :to="`/orgs/${id}/sites/new`" class="c-btn primary">{{ $t('org.console.newSite') }}</NuxtLink>
        <p v-if="!sites.length" class="c-mute">{{ $t('org.console.noSites') }}</p>
        <ul class="c-list">
          <li v-for="s in sites" :key="site(s).id">
            <strong>{{ site(s).title }}</strong>
            <CreatorStatusBadge v-if="s.site_status" :v="s.site_status" />
            <span class="c-mute">{{ s.site_address }}</span>
            <NuxtLink v-if="canEdit" :to="`/lists/${site(s).id}`">{{ $t('org.console.edit') }}</NuxtLink>
            <NuxtLink :to="`/s/${site(s).slug}`">{{ $t('org.console.publicPage') }}</NuxtLink>
          </li>
        </ul>
      </section>
    </template>
  </main>
</template>

<style scoped>
.c-list{list-style:none;padding:0;margin:12px 0 0;display:grid;gap:12px}
.c-list li{display:flex;flex-wrap:wrap;gap:4px 12px;align-items:baseline}
</style>
