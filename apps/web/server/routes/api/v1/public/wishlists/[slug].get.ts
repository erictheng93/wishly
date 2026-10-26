// S1 預覽專用：僅在 NUXT_DEMO=1（見 wrangler.jsonc）時回傳固定資料，正式環境走 apiBase 指向的真實 API，這支不會被呼叫
// 三種情境對應首頁 landing：gift（送禮）/ birthday（生日，demo 為別名）/ wedding（婚禮）
type Raw = Record<string, any>
const q = (id: string, title: string, priority: string, need: number, got: number, price: number | null, extra: Raw = {}): Raw =>
  ({ id, title, priority, qty_needed: need, qty_claimed: got, unit_price_amount: price, ...extra })
// 心願眾籌品項（P2-A）：demo 沒有後端，點「心願眾籌」會因 /me 失敗而導到登入頁
const fund = (id: string, title: string, priority: string, target: number, pledged: number, deadline: string, contributors: [string, number][], extra: Raw = {}): Raw =>
  ({ id, title, priority, qty_needed: 1, qty_claimed: 0, unit_price_amount: null, funding_mode: 'crowdfund', target_points: target, pledged_points: pledged,
    remaining_points: target - pledged, funding_status: 'open', display_status: 'open', funding_deadline: deadline,
    contributors: contributors.map(([display_name, points]) => ({ display_name, points })), ...extra })

const LISTS: Record<string, { title: string, description: string, owner: string, event_date: string, items: Raw[] }> = {
  birthday: {
    title: '小安的生日心願', description: '11 月 8 日生日，大家一起幫我湊一副耳機吧', owner: '小安', event_date: '2026-11-08',
    items: [
      q('i1', '手沖咖啡組', 'high', 1, 1, 2400, { brand: 'Hario', spec: '濾杯 + 細口壺 + 磨豆機' }),
      fund('i2', '降噪耳機', 'high', 8000, 4800, '2026-11-07T15:59:00Z', [['阿凱', 2000], ['匿名朋友', 1800], ['小華', 1000]], { brand: 'Sony', spec: '黑色' }),
      q('i3', '設計的法則', 'medium', 1, 0, 480, { spec: '繁體中文版' }),
      q('i4', '香氛蠟燭', 'low', 2, 1, 350, { brand: 'Jo Malone', spec: '任一款' }),
    ],
  },
  gift: {
    title: 'Mia 的聖誕心願', description: '不知道送什麼的話，看這裡就好，有人要送的話我會自己標記收到', owner: 'Mia', event_date: '2026-12-25',
    items: [
      q('i1', '羊毛圍巾', 'high', 1, 1, 1200, { spec: '燕麥色' }),
      q('i2', '保溫瓶 500ml', 'medium', 1, 0, 900, { brand: 'Stanley', spec: '霧黑' }),
      q('i3', '襪子禮盒', 'low', 3, 1, 300, { spec: 'M 尺寸' }),
      fund('i4', '拍立得相機', 'high', 5000, 3500, '2026-12-23T15:59:00Z', [['阿哲', 2000], ['小茹', 1000], ['匿名朋友', 500]], { brand: 'Instax', spec: 'Mini 12' }),
    ],
  },
  wedding: {
    title: '宇軒 & 佩佩的新家清單', description: '1 月 16 日結婚，新家還缺這些，謝謝大家的祝福', owner: '宇軒 & 佩佩', event_date: '2027-01-16',
    items: [
      q('i1', '氣炸鍋 5L', 'high', 1, 0, 3200, { brand: 'Philips' }),
      q('i2', '餐盤組（6 入）', 'medium', 2, 1, 1500, { spec: '白色陶瓷' }),
      q('i3', '雙人床包組', 'medium', 1, 1, 2200, { spec: '灰藍色 6×7 尺' }),
      fund('i4', '掃地機器人', 'high', 12000, 7200, '2027-01-14T15:59:00Z', [['舅舅', 3000], ['大學同學', 2400], ['匿名朋友', 1800]], { brand: 'Roborock' }),
      q('i5', '玻璃保鮮盒', 'low', 3, 0, 600),
    ],
  },
}

export default defineEventHandler((event) => {
  if (!useRuntimeConfig(event).demo) throw createError({ statusCode: 404 })
  const slug = getRouterParam(event, 'slug') ?? ''
  const key = slug === 'demo' ? 'birthday' : slug
  const l = LISTS[key]
  if (!l) throw createError({ statusCode: 404 })
  const items: Raw[] = l.items.map(i => ({ funding_mode: 'quantity', brand: null, spec: null, image_url: null, product_url: null, ...i,
    qty_remaining: i.qty_needed - i.qty_claimed, is_fully_claimed: i.qty_claimed >= i.qty_needed,
    progress_percent: i.target_points ? Math.floor(i.pledged_points / i.target_points * 100) : Math.round(i.qty_claimed / i.qty_needed * 100) }))
  const claimed = items.reduce((a, i) => a + i.qty_claimed, 0), needed = items.reduce((a, i) => a + i.qty_needed, 0)
  return { id: `w-${key}`, slug, type: 'registry', status: 'active', title: l.title, description: l.description,
    cover_image_url: null, cover_preset: key === 'wedding' ? 'wedding' : key, event_date: l.event_date, owner: { id: 'u1', display_name: l.owner }, surprise_mode: false, claimers_visible: false,
    completion: { item_count: items.length, fulfilled_count: items.filter(i => i.is_fully_claimed).length, completion_pct: Math.round(claimed / needed * 100) },
    items, updated_at: new Date().toISOString() }
})
