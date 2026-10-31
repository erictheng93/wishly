<script setup lang="ts">
const { user } = useAuth()
const { unread, refresh } = useInbox()
const route = useRoute()
const { t } = useI18n()
// 進頁、換頁時取一次（不輪詢）；未登入不打 API
watch(() => [route.fullPath, user.value?.id], () => { if (user.value) refresh() }, { immediate: true })
const label = computed(() => unread.value > 0 ? `${t('common.notifications')}（${t('common.unreadCount', { n: unread.value })}）` : t('common.notifications'))
</script>
<template>
  <NuxtLink v-if="user" to="/me/notifications" class="c-bell" :aria-label="label">
    <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="miter" aria-hidden="true"><path d="M6 17V11a6 6 0 0 1 12 0v6l2 2H4l2-2zM10 21h4" /></svg>
    <b v-if="unread > 0" class="c-bell-n" aria-hidden="true">{{ unread > 99 ? '99+' : unread }}</b>
  </NuxtLink>
</template>
