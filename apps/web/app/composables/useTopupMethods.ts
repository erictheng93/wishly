// 後端開放的儲值方式（/public/status 的 topup_methods，空陣列 = 未開放）；與 useOrgsEnabled 共用 'pub-status'，取不到時視為未開放（fail-closed）
export function useTopupMethods() {
  const { public: { apiBase } } = useRuntimeConfig()
  const { data, status } = useAsyncData('pub-status', () =>
    $fetch<{ orgs_enabled?: boolean, topup_methods?: string[] }>(`${apiBase}/api/v1/public/status`).catch(() => null))
  return { methods: computed(() => data.value?.topup_methods ?? []), ready: computed(() => status.value === 'success' || status.value === 'error') }
}
