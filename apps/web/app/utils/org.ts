// 組織申請表共用：統編檢查、步驟判斷、欄位可編輯性
export const ORG_TYPES = ['association', 'foundation', 'religious', 'other'] as const

// 統編檢查碼（與後端同演算法）：權重 1,2,1,2,1,2,4,1 各積的數字和，可被 5 整除；第 7 碼為 7 時差 1 也算
export function taxIdOk(s: string): boolean {
  if (!/^\d{8}$/.test(s)) return false
  const w = [1, 2, 1, 2, 1, 2, 4, 1]
  const sum = [...s].reduce((a, c, i) => { const p = Number(c) * w[i]!; return a + Math.floor(p / 10) + (p % 10) }, 0)
  return sum % 5 === 0 || (s[6] === '7' && (sum + 1) % 5 === 0)
}

export type OrgForm = Record<string, any> & { verification_status?: string, flagged_fields?: string[], documents?: { kind: string }[] }

// 依缺漏欄位決定從第幾步繼續
export function stepOf(o: OrgForm): 1 | 2 | 3 | 4 {
  if (!o.legal_name || !o.display_name || !o.org_type || !taxIdOk(o.tax_id ?? '') || !o.registration_no || !o.authority) return 1
  if (!o.rep_name || !o.contact_name || !o.contact_phone || !o.public_email) return 2
  const kinds = new Set((o.documents ?? []).map(d => d.kind))
  if (!kinds.has('registration') || !kinds.has('permit')) return 3
  return 4
}

// needs_changes 只開放被點名的欄位；草稿全開；其餘狀態唯讀
export function editable(o: OrgForm, field: string): boolean {
  if (o.verification_status === 'needs_changes') return !!o.flagged_fields?.includes(field)
  return !o.verification_status || o.verification_status === 'draft'
}
