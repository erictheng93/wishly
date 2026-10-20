// 從網址 query 取新增品項的預填值；query 不可信，字串限長、網址限 http(s)、價格限非負整數
const MAX = { title: 120, brand: 100, spec: 200, product_url: 2000 } as const
export type ItemPrefill = { title?: string; brand?: string; spec?: string; product_url?: string; unit_price_amount?: number; catalog_ref?: string }

export function sanitizeItemPrefill(q: Record<string, unknown>): ItemPrefill {
  const out: ItemPrefill = {}
  for (const k of Object.keys(MAX) as (keyof typeof MAX)[]) {
    const v = q[k]
    if (typeof v !== 'string') continue
    const s = v.trim().slice(0, MAX[k])
    if (!s) continue
    if (k === 'product_url') {
      try { if (!['http:', 'https:'].includes(new URL(s).protocol)) continue } catch { continue }
    }
    out[k] = s
  }
  const p = q.unit_price_amount
  if (typeof p === 'string' && /^\d{1,9}$/.test(p.trim())) out.unit_price_amount = Number(p.trim())
  const r = q.ref // 精選商品 id（後端 catalog_ref 同規則）
  if (typeof r === 'string' && /^[A-Za-z0-9_-]{1,64}$/.test(r)) out.catalog_ref = r
  return out
}
