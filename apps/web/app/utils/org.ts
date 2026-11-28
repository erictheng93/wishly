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
export function stepOf(o: OrgForm, permits: unknown[] = []): 1 | 2 | 3 | 4 {
  if (!o.legal_name || !o.display_name || !o.org_type || !taxIdOk(o.tax_id ?? '') || !o.registration_no || !o.authority) return 1
  if (!o.rep_name || !o.contact_name || !o.contact_phone || !o.public_email) return 2
  const kinds = new Set((o.documents ?? []).map(d => d.kind))
  if (!kinds.has('registration') || !kinds.has('permit') || !permits.length) return 3
  return 4
}

// needs_changes 只開放被點名的欄位；草稿全開；其餘狀態唯讀
export function editable(o: OrgForm, field: string): boolean {
  if (o.verification_status === 'needs_changes') return !!o.flagged_fields?.includes(field)
  return !o.verification_status || o.verification_status === 'draft'
}

// 申請第 3 步（文件與許可）能否往下：至少 1 份登記證與 1 筆許可；已選許可檔卻沒按「新增」要先擋。回傳錯誤 i18n key
export function permitStepError(o: OrgForm | null, permits: unknown[], pendingFile: boolean): string | null {
  if (pendingFile) return 'org.apply.permitPending'
  const hasReg = (o?.documents ?? []).some(d => d.kind === 'registration')
  return hasReg && permits.length ? null : 'org.apply.docsRequired'
}

export type NextStep = { key: 'event' | 'site' | 'live', done: boolean }
// 組織主控台「接下來」三步，全由該頁已載入的事件與據點推導；上線＝據點 status 與 site_status 皆 active
export function nextSteps(events: unknown[], sites: { status?: string, site_status?: string }[]): NextStep[] {
  return [
    { key: 'event', done: events.length > 0 },
    { key: 'site', done: sites.length > 0 },
    { key: 'live', done: sites.some(s => s.status === 'active' && s.site_status === 'active') },
  ]
}
