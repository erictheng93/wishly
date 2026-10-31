// 後台數據分析的純函式。null = 後端小樣本抑制（1–4 人），顯示「<5」，且不納入百分比計算。
export type Count = number | null

export const SUPPRESSED = '<5'
const t = (k: string) => useNuxtApp().$i18n.t(k)
const numLocale = () => ({ en: 'en', vi: 'vi', id: 'id' } as Record<string, string>)[useNuxtApp().$i18n.locale.value] ?? 'zh-TW'
export const fmtCount = (v: Count | undefined): string => v == null ? SUPPRESSED : v.toLocaleString(numLocale())

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
  return ents.map(([key, v]) => ({ key, label: Object.hasOwn(labels, key) ? labels[key]! : key, v, share: pct(v, known), w: v && max ? v / max * 100 : 0 }))
}

export interface FunnelStep { key: string; label: string; v: number; step: string; w: number }
const FUNNEL_KEYS = ['signed_up', 'created_list', 'published_list', 'received_claim']
/** 漏斗：step 為相對上一步的轉換率，第一步為「–」；w 相對第一步（0..100）。 */
export function funnelSteps(f: Record<string, number> | undefined): FunnelStep[] {
  const keys = FUNNEL_KEYS
  const top = f?.[keys[0]!] ?? 0
  return keys.map((key, i) => {
    const v = f?.[key] ?? 0
    return { key, label: t(`creator.analytics.funnel.${key}`), v, step: i ? pct(v, f?.[keys[i - 1]!] ?? 0) : '–', w: top > 0 ? Math.min(100, v / top * 100) : 0 }
  })
}

/** 平均（1 位小數字串）；空陣列回「–」。 */
export const mean = (xs: number[]): string => xs.length ? (xs.reduce((a, b) => a + b, 0) / xs.length).toFixed(1) : '–'

export interface RetentionRow { key: string; label: string; cohort: number; retained: number; pct: string; w: number }
const RETENTION_KEYS = ['d1', 'd7', 'd30']
/** 留存列；缺 retention（舊 API）或缺鍵視為 0。cohort 為 0 時 pct 為「–」、條寬 0。 */
export function retentionRows(r: Record<string, { cohort?: number; retained?: number } | undefined> | undefined): RetentionRow[] {
  return RETENTION_KEYS.map((key) => {
    const label = t(`creator.analytics.retention.${key}`)
    const cohort = r?.[key]?.cohort ?? 0, retained = r?.[key]?.retained ?? 0
    return { key, label, cohort, retained, pct: pct(retained, cohort), w: cohort > 0 ? Math.min(100, retained / cohort * 100) : 0 }
  })
}

// getter：讀取時才翻譯（語系隨請求 / 切換而變）
const grp = (g: string, ks: string[]) => Object.fromEntries(ks.map(k => [k, t(`creator.analytics.${g}.${k}`)]))
export const L = {
  get claimStatus() { return grp('claimStatus', ['reserved', 'purchased', 'delivered', 'cancelled', 'expired']) },
  get device() { return grp('device', ['mobile', 'tablet', 'desktop']) },
  get viewOther() { return grp('viewOther', ['direct', 'unknown']) },
  get reason() { return grp('reason', ['scam', 'inappropriate', 'copyright', 'personal_info', 'other']) },
  provider: { email: 'Email', line: 'LINE', google: 'Google' },
  get wishlistType() { return grp('wishlistType', ['personal', 'registry', 'relief']) },
  get wishlistVisibility() { return grp('wishlistVisibility', ['public', 'friends', 'selected', 'private']) },
  get shareLevel() { return grp('shareLevel', ['public', 'friends', 'private']) },
} as Record<string, Record<string, string>>
