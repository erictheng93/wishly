// 金流後台（/admin/finance）的區間、分頁與指標小工具
export const FINANCE_RANGES = [1, 7, 30, 90, 365] as const
export const FINANCE_TABS = ['overview', 'transactions', 'payers', 'channels', 'risk'] as const
export type FinanceTab = typeof FINANCE_TABS[number]

/** 網址 ?tab= 轉分頁；未知值回 overview */
export const financeTab = (q: unknown): FinanceTab =>
  (FINANCE_TABS as readonly unknown[]).includes(q) ? q as FinanceTab : 'overview'

/** 與前期的變動百分比（原始數字，由元件取整）；前期為 0 回 null */
export const delta = (cur: number, prev: number): number | null => prev === 0 ? null : (cur - prev) / prev * 100

/** 平均每人；沒有付費用戶回 null */
export const perPayer = (gross: number, payers: number): number | null => payers === 0 ? null : gross / payers

/** 0–1 比率轉百分比字串；null 顯示 – */
export const fmtRate = (r: number | null | undefined): string => r == null ? '–' : `${(r * 100).toFixed(1)}%`

/** 日期一律以台北時間顯示；locale 預設取目前介面語言 */
export const fmtDay = (iso: string, locale: string = intlLocale()): string =>
  new Date(iso).toLocaleDateString(locale, { timeZone: 'Asia/Taipei' })

/** 付款方式名稱：優先用儲值頁的翻譯，其餘（line_pay、apple_pay）用金流後台的；都沒有才顯示代碼 */
export const methodLabel = (m: string, t: (k: string) => string, te: (k: string) => boolean): string =>
  te(`me.topup.method.${m}`) ? t(`me.topup.method.${m}`) : te(`admin.finance.method.${m}`) ? t(`admin.finance.method.${m}`) : m
