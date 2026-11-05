<script setup lang="ts">
defineProps<{ title: string; back?: string }>()
const { user } = useAuth()
const myPage = computed(() => user.value ? profilePath(user.value) : '/settings')
const { current, orgs } = useOrgMode()
const orgName = computed(() => orgs.value.find(o => o.id === current.value)?.name)
const { count, refresh } = useFriendRequests()
// 進頁、換頁、切回分頁時更新好友申請數（不輪詢）
const onVisible = () => { if (user.value && document.visibilityState === 'visible') refresh() }
const route = useRoute()
onMounted(() => document.addEventListener('visibilitychange', onVisible))
onBeforeUnmount(() => document.removeEventListener('visibilitychange', onVisible))
watch(() => [route.fullPath, user.value?.id], () => { if (user.value) refresh() }, { immediate: true })
</script>
<template>
  <header class="c-head">
    <NuxtLink v-if="back" :to="back" class="c-btn plain c-back" :aria-label="$t('common.back')"><UiIcon name="back" /></NuxtLink>
    <NuxtLink to="/" class="c-brand">Wishly</NuxtLink>
    <span v-if="orgName" class="c-org"><span class="c-org-k">{{ $t('org.badge') }}：</span>{{ orgName }}</span>
    <LocaleSwitch /><ThemeToggle />
    <slot />
    <CreatorBell /><CreatorUserMenu />
  </header>
  <!-- 頁面標題移出 sticky 頁首：編輯式大標，頁首只留品牌與動作 -->
  <div class="c-title"><h1>{{ title }}</h1></div>
  <OnboardingGuide />
  <!-- 桌面（≥1280）側欄：取代各頁重複的導覽按鈕 -->
  <nav v-if="user" class="c-rail" :aria-label="$t('common.mainMenu')">
    <NuxtLink to="/dashboard">{{ $t('common.myLists') }}</NuxtLink>
    <NuxtLink to="/friends">{{ $t('common.friends') }}<CreatorCountBadge :n="count" /></NuxtLink>
    <NuxtLink :to="myPage">{{ $t('common.profile') }}</NuxtLink>
    <NuxtLink to="/me/wallet">{{ $t('common.points') }}</NuxtLink>
    <NuxtLink to="/settings">{{ $t('common.settings') }}</NuxtLink>
    <NuxtLink v-if="user.is_staff" to="/admin">{{ $t('common.admin') }}</NuxtLink>
  </nav>
</template>
<style scoped>
.c-org{padding:0 8px;line-height:22px;background:var(--fg);color:var(--bg);font:600 11px/22px var(--mono);letter-spacing:.06em;max-width:12em;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;flex:0 1 auto;min-width:0}
/* 窄手機：組織身分不藏（操作是以組織名義進行），改成頁首下方一條細列；前綴視覺隱藏、螢幕閱讀器仍讀得到 */
@media (max-width:479px){
  .c-org{order:99;flex-basis:100%;display:flex;align-items:center;gap:8px;max-width:none;margin:0 -20px;padding:0 20px;border-top:1px solid var(--line);background:transparent;color:var(--fg);line-height:28px}
  .c-org::before{content:"";width:6px;height:6px;background:var(--red)}
  .c-org-k{position:absolute;width:1px;height:1px;overflow:hidden;clip-path:inset(50%);white-space:nowrap}
}
</style>
<style>
.c-back{margin-left:-10px}
</style>
