<script setup lang="ts">
// 客服信箱連結：主旨帶情境、內文帶帳號 ID（已登入時），營運可直接在 /admin 查人。
// 後面接站內客服表單（未登入也能用）；error.vue 等不宜導頁處以 form=false 關掉
const props = withDefaults(defineProps<{ topic?: string, form?: boolean }>(), { form: true })
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
  <a :href="href" data-testid="support-link">{{ email }}</a><template v-if="form">{{ ' ' }}<NuxtLink to="/support" data-testid="support-form-link">{{ $t('common.supportForm') }}</NuxtLink></template>
</template>
