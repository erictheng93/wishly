// 後台數據分析的純函式。null = 後端小樣本抑制（1–4 人），顯示「<5」，且不納入百分比計算。
export type Count = number | null

export const SUPPRESSED = '<5'
export const fmtCount = (v: Count | undefined): string => v == null ? SUPPRESSED : v.toLocaleString('zh-TW')

/** n/d 的百分比字串；任一為 null/非有限、或分母 ≤ 0 時回「–」。 */
export function pct(n: Count | undefined, d: Count | undefined, digits = 0): string {
  if (n == null || d == null || !Number.isFinite(n) || !Number.isFinite(d) || d <= 0) return '–'
  return `${(n / d * 100).toFixed(digits)}%`
}

export interface BarRow { key: string; label: string; v: Count; share: string; w: number }

/** 物件 → 長條列：依值遞減（null 排最後），share 以非 null 總和為分母，w 為 0..100 的條寬（null 為 0）。 */
export function barRows(obj: Record<string, Count> | undefined, labels: Record<string, string> = {}, sort = true): BarRow[] {
  const ents = Object.entries(obj ?? {})
  const known = ents.reduce((s, [, v]) => s + (v ?? 0), 0)
  const max = Math.max(0, ...ents.map(([, v]) => v ?? 0))
  if (sort) ents.sort((a, b) => (b[1] ?? -1) - (a[1] ?? -1))
  return ents.map(([key, v]) => ({ key, label: labels[key] ?? key, v, share: pct(v, known), w: v && max ? v / max * 100 : 0 }))
}

export interface FunnelStep { key: string; label: string; v: number; step: string; w: number }
const FUNNEL_LABEL: Record<string, string> = { signed_up: '註冊', created_list: '建立清單', published_list: '發布清單', received_claim: '收到捐助' }
/** 漏斗：step 為相對上一步的轉換率，第一步為「–」；w 相對第一步（0..100）。 */
export function funnelSteps(f: Record<string, number> | undefined): FunnelStep[] {
  const keys = Object.keys(FUNNEL_LABEL)
  const top = f?.[keys[0]!] ?? 0
  return keys.map((key, i) => {
    const v = f?.[key] ?? 0
    return { key, label: FUNNEL_LABEL[key]!, v, step: i ? pct(v, f?.[keys[i - 1]!] ?? 0) : '–', w: top > 0 ? Math.min(100, v / top * 100) : 0 }
  })
}

export const L = {
  claimStatus: { reserved: '已認領', purchased: '已購買', delivered: '已送達', cancelled: '已取消', expired: '已逾期' },
  actor: { guest: '訪客', user: '會員' },
  reason: { scam: '疑似詐騙', inappropriate: '不當內容', copyright: '侵害著作權', personal_info: '洩漏個人資料', other: '其他' },
  provider: { email: 'Email', line: 'LINE', google: 'Google' },
  wishlistType: { personal: '個人心願', registry: '禮物登記', relief: '救助' },
  wishlistVisibility: { link: '連結', private: '私人', public: '公開' },
  shareLevel: { public: '公開', friends: '好友', private: '私人' },
} as Record<string, Record<string, string>>
