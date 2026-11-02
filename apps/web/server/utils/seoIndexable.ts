import type { H3Event } from 'h3'

// 只有正式網域給收錄：示範站（NUXT_DEMO）與 *.pages.dev 預覽一律擋，避免重複內容搶排名
export function seoIndexable(event: H3Event) {
  const c = useRuntimeConfig(event)
  return !c.demo && getRequestURL(event).host === new URL(c.public.siteUrl).host
}
