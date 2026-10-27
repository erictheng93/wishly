<script setup lang="ts">
// 公益組織介紹頁（訪客可看，SSR）；CTA 未登入先走登入，回來直接進申請表
const { t } = useI18n()
useSeoMeta({ title: () => t('org.seoTitle'), description: () => t('org.seoDesc') })
if (!(await useOrgsEnabled()).value) throw createError({ statusCode: 404, fatal: true })
const { user } = useAuth()
const hint = useAuthHint()
const cta = computed(() => (user.value || hint.value ? '/org/apply' : '/login?redirect=/org/apply'))
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <h1>{{ $t('org.heading') }}</h1>
    <ul class="x-pts">
      <li v-for="n in 4" :key="n" class="g-card">{{ $t(`org.p${n}`) }}</li>
    </ul>
    <NuxtLink :to="cta" class="g-btn">{{ $t('org.cta') }}</NuxtLink>
    <h2>{{ $t('org.prepTitle') }}</h2>
    <ul>
      <li v-for="n in 3" :key="n">{{ $t(`org.prep${n}`) }}</li>
    </ul>
  </main>
</template>

<style scoped>
.x-pts{list-style:none;padding:0;margin:16px 0 24px}
</style>
