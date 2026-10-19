// 營運後台（現金退款 / 欠款 / 儲值爭議）的動作對應：依目前狀態提供合法動作、組請求 body、驗證必填理由。
// 契約：PATCH /admin/cash-refunds/{id}（processing | completed | rejected，rejected 必填 reject_reason；
// 狀態機 requested → processing → completed，requested / processing → rejected）、PATCH /admin/wallet-debts/{id}（settled | written_off，note 必填）、
// POST /admin/topups/{id}/dispute（open | won | lost，note 選填）。

export type RefundTo = 'processing' | 'completed' | 'rejected'
export const CASH_REFUND_STATUSES = ['requested', 'processing', 'completed', 'rejected'] as const

/** 後端狀態機：requested → processing | rejected；processing → completed | rejected；其餘不可再動 */
export function cashRefundActions(status: string): RefundTo[] {
  if (status === 'requested') return ['processing', 'rejected']
  if (status === 'processing') return ['completed', 'rejected']
  return []
}

/** 回 { body } 或 { error }（i18n key）。rejected 必填理由；其餘不送 reject_reason */
export function cashRefundPatch(to: RefundTo, reason: string): { body?: Record<string, string>, error?: string } {
  if (to !== 'rejected') return { body: { status: to } }
  const r = reason.trim()
  return r ? { body: { status: to, reject_reason: r } } : { error: 'admin.cashRefunds.reasonRequired' }
}

export type DebtTo = 'settled' | 'written_off'
export const DEBT_STATUSES = ['open', 'settled', 'written_off'] as const
export const debtActions = (status: string): DebtTo[] => (status === 'open' ? ['settled', 'written_off'] : [])
export function debtPatch(to: DebtTo, note: string): { body?: Record<string, string>, error?: string } {
  const n = note.trim()
  return n ? { body: { status: to, note: n } } : { error: 'admin.walletDebts.noteRequired' }
}

export type DisputeAction = 'open' | 'won' | 'lost'
export const TOPUP_FILTERS = ['disputed', 'paid', 'charged_back', ''] as const
/** paid 可標記爭議；disputed 可判勝訴 / 敗訴；其餘（含 charged_back）不可再動 */
export function disputeActions(status: string): DisputeAction[] {
  if (status === 'paid') return ['open']
  if (status === 'disputed') return ['won', 'lost']
  return []
}
export const disputeBody = (action: DisputeAction, note: string) => {
  const n = note.trim()
  return n ? { action, note: n } : { action }
}
/** 爭議動作後列的狀態與錢包狀態：open / lost 後端會凍結錢包；won 是否解凍要看有無其他爭議，未知回 null */
export const disputeOutcome = (action: DisputeAction): { status: string, walletFrozen: boolean | null } =>
  action === 'open' ? { status: 'disputed', walletFrozen: true } : action === 'lost' ? { status: 'charged_back', walletFrozen: true } : { status: 'paid', walletFrozen: null }

/** 列表回應容忍 data / topups 兩種欄位名（admin 其他列表是 data；topups 契約寫 topups） */
export const listRows = (r: any): any[] => r?.data ?? r?.topups ?? []
