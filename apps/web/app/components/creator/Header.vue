<script setup lang="ts">
defineProps<{ title: string; back?: string }>()
const { user } = useAuth()
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
    <NuxtLink v-if="back" :to="back" class="c-btn plain" :aria-label="$t('common.back')">‹</NuxtLink>
    <h1>{{ title }}</h1>
    <LocaleSwitch /><ThemeToggle />
    <slot />
    <CreatorUserMenu />
  </header>
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
