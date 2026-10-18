// 探索頁示範資料：同 [slug].get.ts，只在 NUXT_DEMO=1 的建置回傳；正式環境走真實 API，這支不會被呼叫
const CARDS = [
  { slug: 'demo', title: '小安的生日心願', type: 'registry', event_date: '2026-11-08', owner: { display_name: '小安', handle: null }, item_count: 4, completion_pct: 62 },
  { slug: 'gift', title: 'Mia 的聖誕心願', type: 'registry', event_date: '2026-12-25', owner: { display_name: 'Mia', handle: null }, item_count: 4, completion_pct: 50 },
  { slug: 'wedding', title: '宇軒 & 佩佩的新家清單', type: 'registry', event_date: '2027-01-16', owner: { display_name: '宇軒 & 佩佩', handle: null }, item_count: 5, completion_pct: 40 },
]
export default defineEventHandler((event) => {
  if (!useRuntimeConfig(event).demo) throw createError({ statusCode: 404 })
  const type = String(getQuery(event).type ?? '')
  return { wishlists: CARDS.map(c => ({ ...c, cover_image_url: null })).filter(c => !type || c.type === type), next_page: null }
})
