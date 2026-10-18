<script setup lang="ts">
defineProps<{ title: string; back?: string }>()
const { user } = useAuth()
</script>
<template>
  <header class="c-head">
    <NuxtLink v-if="back" :to="back" class="c-btn plain" :aria-label="$t('common.back')">‹</NuxtLink>
    <h1>{{ title }}</h1>
    <NuxtLink v-if="user?.handle" :to="`/u/${user.handle}`" class="c-btn c-nav-dup">{{ $t('common.profile') }}</NuxtLink>
    <NuxtLink v-if="user" to="/friends" class="c-btn c-nav-dup">{{ $t('common.friends') }}</NuxtLink>
    <LocaleSwitch /><ThemeToggle />
    <slot />
  </header>
  <!-- 桌面（≥1280）側欄：取代各頁重複的導覽按鈕 -->
  <nav v-if="user" class="c-rail" :aria-label="$t('common.mainMenu')">
    <NuxtLink to="/dashboard">{{ $t('common.myLists') }}</NuxtLink>
    <NuxtLink to="/friends">{{ $t('common.friends') }}</NuxtLink>
    <NuxtLink v-if="user.handle" :to="`/u/${user.handle}`">{{ $t('common.profile') }}</NuxtLink>
    <NuxtLink to="/me/wallet">{{ $t('common.points') }}</NuxtLink>
    <NuxtLink to="/settings">{{ $t('common.settings') }}</NuxtLink>
    <NuxtLink v-if="user.is_staff" to="/admin">{{ $t('common.admin') }}</NuxtLink>
  </nav>
</template>
