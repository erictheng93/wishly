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
