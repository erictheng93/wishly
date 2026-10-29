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
</script>

<template>
  <main class="c-page oh">
    <CreatorHeader :title="org?.display_name || $t('org.console.title')" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="loading" class="c-mute" role="status">{{ $t('org.apply.loading') }}</p>
    <template v-else-if="org">
      <nav class="c-row oh-nav">
        <NuxtLink v-if="role === 'owner'" :to="`/orgs/${id}/members`" class="c-btn">{{ $t('org.console.members') }}</NuxtLink>
      </nav>

      <section class="c-card oh-sec">
        <h2 class="c-h2">{{ $t('org.console.events') }}<b class="oh-count" aria-hidden="true">{{ String(events.length).padStart(2, '0') }}</b></h2>
        <NuxtLink v-if="canEdit" :to="`/orgs/${id}/events/new`" class="c-btn primary">{{ $t('org.console.newEvent') }}</NuxtLink>
        <p v-if="!events.length" class="c-mute">{{ $t('org.console.noEvents') }}</p>
        <ul class="c-list oh-list">
          <li v-for="e in events" :key="e.id">
            <strong>{{ e.name }}</strong>
            <span class="c-mute">{{ e.occurred_on }}　{{ (e.counties ?? []).join('、') }}</span>
          </li>
        </ul>
      </section>

      <section class="c-card oh-sec">
        <h2 class="c-h2">{{ $t('org.console.sites') }}<b class="oh-count" aria-hidden="true">{{ String(sites.length).padStart(2, '0') }}</b></h2>
        <NuxtLink v-if="canEdit" :to="`/orgs/${id}/sites/new`" class="c-btn primary">{{ $t('org.console.newSite') }}</NuxtLink>
        <p v-if="!sites.length" class="c-mute">{{ $t('org.console.noSites') }}</p>
        <ul class="c-list oh-list">
          <li v-for="s in sites" :key="s.id">
            <strong>{{ s.title }}</strong>
            <CreatorStatusBadge v-if="s.site_status" :v="s.site_status" />
            <span class="c-mute">{{ s.site_address }}</span>
            <span class="oh-acts">
              <NuxtLink v-if="canEdit" :to="`/lists/${s.id}/edit`">{{ $t('org.console.edit') }}</NuxtLink>
              <NuxtLink :to="`/orgs/${id}/sites/${s.id}/receipts`">{{ $t('org.console.receipts') }}</NuxtLink>
              <NuxtLink :to="`/s/${s.slug}`">{{ $t('org.console.publicPage') }}</NuxtLink>
            </span>
          </li>
        </ul>
      </section>
    </template>
  </main>
</template>

<style scoped>
.oh-nav{margin-bottom:16px}
.oh-sec{padding:0;border-top:2px solid var(--ink-line);margin-bottom:24px}
.oh-sec>*{margin-left:18px;margin-right:18px}
.oh-sec>.c-h2{margin:0;padding:14px 18px;background:var(--bg-alt);border-bottom:1px solid var(--line)}
.oh-count{margin-left:auto;font:700 13px var(--mono);letter-spacing:.08em;color:var(--mute);border:1px solid var(--line);padding:2px 8px;background:var(--card)}
.oh-sec>.c-btn{margin-top:16px}
.oh-sec>.c-mute{margin-top:16px;margin-bottom:18px}
.oh-list{list-style:none;padding:0;margin:4px 18px 0!important;display:block}
.oh-list li{display:grid;grid-template-columns:auto 1fr;gap:4px 12px;align-items:center;padding:14px 0;border-top:1px solid var(--line);animation:none}
.oh-list li:first-child{border-top:0}
.oh-list li>strong{font-size:17px;grid-column:1}
.oh-list li>.c-mute{grid-column:1/-1;font:500 12px/1.5 var(--mono);letter-spacing:.03em}
.oh-list li>.c-badge{grid-column:2;justify-self:start}
.oh-acts{grid-column:1/-1;display:flex;flex-wrap:wrap;margin-top:2px}
.oh-acts a{padding:0 12px 0 0;margin-right:12px;border-right:1px solid var(--line);font-size:14px;font-weight:700;text-decoration:underline;text-decoration-color:var(--red);text-underline-offset:4px}
.oh-acts a:last-child{border-right:0}
@media (min-width:768px){
  .oh-list li{grid-template-columns:auto auto 1fr}
  .oh-list li>.c-badge{grid-column:2}
  .oh-acts{grid-column:3;grid-row:1/3;justify-self:end;margin:0}
  .oh-list li>.c-mute{grid-column:1/3}
}
</style>
