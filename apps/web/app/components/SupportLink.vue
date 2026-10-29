<script setup lang="ts">
// 客服信箱連結：主旨帶情境、內文帶帳號 ID（已登入時），營運可直接在 /admin 查人
const props = defineProps<{ topic?: string }>()
const { t } = useI18n()
const email = useRuntimeConfig().public.supportEmail
const user = useState<any>('auth-user', () => null)
const href = computed(() => {
  const subject = props.topic ? `${t('common.supportSubject')}：${props.topic}` : t('common.supportSubject')
  const body = user.value?.id ? t('common.supportBody', { id: user.value.id }) : ''
  return `mailto:${email}?subject=${encodeURIComponent(subject)}${body ? `&body=${encodeURIComponent(body)}` : ''}`
})
</script>
<template>
  <a :href="href" data-testid="support-link">{{ email }}</a>
</template>
