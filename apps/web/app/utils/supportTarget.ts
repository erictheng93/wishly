/** 從貼上的分享連結（…/s/{slug}）或純代稱取出清單 slug；格式不符回 null。 */
export const parseListSlug = (input: string): string | null => {
  const s = input.trim()
  return s.match(/^[\w-]+$/)?.[0] ?? s.match(/^https?:\/\/[^/\s]+\/s\/([\w-]+)\/?(?:[?#]\S*)?$/)?.[1] ?? null
}
