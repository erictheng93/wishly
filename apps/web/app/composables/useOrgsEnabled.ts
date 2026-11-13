// 公益組織功能是否開啟（後端 ORGS_ENABLED）；與 SystemBanner 共用 /public/status，取不到時視為關閉
export async function useOrgsEnabled() {
  const { public: { apiBase } } = useRuntimeConfig()
  const { data } = await useAsyncData('pub-status', () =>
    fetchPublicStatus(apiBase).catch(() => null))
  return computed(() => data.value?.orgs_enabled === true)
}
