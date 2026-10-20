import { catalogQuery, type CatalogFilter, type CatalogProduct } from '~/utils/catalog'
// 商品目錄清單（SSR 第一頁 + 「載入更多」）；篩選變動時重抓並清掉後續頁。公開 API：不帶憑證（CDN 快取）
export async function useCatalog(key: string, filter: () => CatalogFilter) {
  const { public: { apiBase } } = useRuntimeConfig()
  const url = `${apiBase}/api/v1/public/catalog/products`
  const { data, error, refresh } = await useAsyncData(key, () => $fetch<any>(url, { query: catalogQuery(filter()) }), { watch: [filter] })
  const extra = ref<CatalogProduct[]>([]), extraNext = ref<number | null | undefined>(undefined), busy = ref(false), moreErr = ref(false)
  watch(data, () => { extra.value = []; extraNext.value = undefined })
  const products = computed<CatalogProduct[]>(() => [...(data.value?.products ?? []), ...extra.value])
  const next = computed(() => extraNext.value !== undefined ? extraNext.value : data.value?.next_page ?? null)
  async function more() {
    busy.value = true; moreErr.value = false
    try {
      const r = await $fetch<any>(url, { query: catalogQuery(filter(), next.value) })
      extra.value = [...extra.value, ...r.products]; extraNext.value = r.next_page
    } catch { moreErr.value = true } finally { busy.value = false }
  }
  return { products, next, error, refresh, more, busy, moreErr }
}
