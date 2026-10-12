<script setup lang="ts">
defineProps<{ title: string; back?: string }>()
const { user } = useAuth()
</script>
<template>
  <header class="c-head">
    <NuxtLink v-if="back" :to="back" class="c-btn plain" aria-label="返回">‹</NuxtLink>
    <h1>{{ title }}</h1>
    <NuxtLink v-if="user?.handle" :to="`/u/${user.handle}`" class="c-btn c-nav-dup">個人頁</NuxtLink>
    <NuxtLink v-if="user" to="/friends" class="c-btn c-nav-dup">好友</NuxtLink>
    <slot />
  </header>
  <!-- 桌面（≥1280）側欄：取代各頁重複的導覽按鈕 -->
  <nav v-if="user" class="c-rail" aria-label="主選單">
    <NuxtLink to="/dashboard">我的清單</NuxtLink>
    <NuxtLink to="/friends">好友</NuxtLink>
    <NuxtLink v-if="user.handle" :to="`/u/${user.handle}`">個人頁</NuxtLink>
    <NuxtLink to="/me/wallet">點數</NuxtLink>
    <NuxtLink to="/settings">設定</NuxtLink>
    <NuxtLink v-if="user.is_staff" to="/admin">後台</NuxtLink>
  </nav>
</template>
