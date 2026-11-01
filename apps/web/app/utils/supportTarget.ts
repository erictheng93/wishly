/** 從貼上的分享連結（…/s/{slug}）或純代稱取出清單 slug；格式不符回 null。 */
export const parseListSlug = (input: string): string | null => {
  const s = input.trim()
  return s.match(/^[\w-]+$/)?.[0] ?? s.match(/^https?:\/\/[^/\s]+\/s\/([\w-]+)\/?(?:[?#]\S*)?$/)?.[1] ?? null
}

/** 非同步驗證回來時，輸入框內容是否仍對應該 slug（否則是過期回應，要丟棄）。 */
export const isCurrentSlug = (input: string, slug: string): boolean => parseListSlug(input) === slug
