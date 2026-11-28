<script setup lang="ts">
// 志工掃捐助者認領 QR 的落地頁：用 ?list=（據點分享代碼）從自己所屬組織找出據點，再導到實收頁並帶上 claim/item/qty
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('org.console.receipts'), meta: [{ name: 'robots', content: 'noindex' }] })
const { api } = useApi()
const route = useRoute()
const { user } = useAuth()
const err = ref('')
onMounted(async () => {
  const list = typeof route.query.list === 'string' ? route.query.list : ''
  const scan = receiptPrefill(route.query)
  if (!list || !scan || scan === 'invalid') { err.value = t('org.relief.receiptScanInvalid'); return }
  for (const o of user.value?.orgs ?? []) {
    try {
      const site = (await api(`/orgs/${o.id}/sites`)).find((s: any) => s.slug === list)
      if (site) { await navigateTo({ path: `/orgs/${o.id}/sites/${site.id}/receipts`, query: { claim: scan.claim, item: scan.item, qty: String(scan.qty) } }, { replace: true }); return }
    } catch { /* 該組織無權限或讀取失敗，換下一個 */ }
  }
  err.value = t('org.relief.receiveNoSite')
})
</script>

<template>
  <main class="c-page">
    <CreatorHeader :title="$t('org.console.receipts')" back="/dashboard" />
    <p v-if="err" class="c-err c-errbox" role="alert">{{ err }}</p>
    <p v-else class="c-mute" role="status">{{ $t('org.apply.loading') }}</p>
  </main>
</template>
