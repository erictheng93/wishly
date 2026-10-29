<script setup lang="ts">
defineProps<{ title: string; back?: string }>()
const { user } = useAuth()
const { current, orgs } = useOrgMode()
const orgName = computed(() => orgs.value.find(o => o.id === current.value)?.name)
const { count, refresh } = useFriendRequests()
// 進頁、換頁、切回分頁時更新好友申請數（不輪詢）
const onVisible = () => { if (user.value && document.visibilityState === 'visible') refresh() }
const route = useRoute()
// 標題上方的等寬索引籤：取路由第一段（DASHBOARD / LISTS / ADMIN…），像實驗室標本編號
const kicker = computed(() => `WISHLY / ${(route.path.split('/')[1] || 'home').toUpperCase()}`)
onMounted(() => document.addEventListener('visibilitychange', onVisible))
onBeforeUnmount(() => document.removeEventListener('visibilitychange', onVisible))
watch(() => [route.fullPath, user.value?.id], () => { if (user.value) refresh() }, { immediate: true })
</script>
<template>
  <header class="c-head">
    <NuxtLink v-if="back" :to="back" class="c-btn plain c-back" :aria-label="$t('common.back')">‹</NuxtLink>
    <NuxtLink to="/" class="c-brand">Wishly</NuxtLink>
    <span v-if="orgName" class="c-org">{{ $t('org.badge') }}：{{ orgName }}</span>
    <LocaleSwitch /><ThemeToggle />
    <slot />
    <CreatorUserMenu />
  </header>
  <!-- 頁面標題移出 sticky 頁首：編輯式大標，頁首只留品牌與動作 -->
  <div class="c-title"><small>{{ kicker }}</small><h1>{{ title }}</h1></div>
  <OnboardingGuide />
  <!-- 桌面（≥1280）側欄：取代各頁重複的導覽按鈕 -->
  <nav v-if="user" class="c-rail" :aria-label="$t('common.mainMenu')">
    <NuxtLink to="/dashboard">{{ $t('common.myLists') }}</NuxtLink>
    <NuxtLink to="/friends">{{ $t('common.friends') }}<CreatorCountBadge :n="count" /></NuxtLink>
    <NuxtLink v-if="user.handle" :to="`/u/${user.handle}`">{{ $t('common.profile') }}</NuxtLink>
    <NuxtLink to="/me/wallet">{{ $t('common.points') }}</NuxtLink>
    <NuxtLink to="/settings">{{ $t('common.settings') }}</NuxtLink>
    <NuxtLink v-if="user.is_staff" to="/admin">{{ $t('common.admin') }}</NuxtLink>
  </nav>
</template>
<style scoped>
.c-org{padding:0 8px;line-height:22px;background:var(--fg);color:var(--bg);font:600 11px/22px var(--mono);letter-spacing:.06em;max-width:12em;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
</style>
<style>
.c-back{font-size:24px;margin-left:-10px}
</style>
