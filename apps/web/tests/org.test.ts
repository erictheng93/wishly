import { describe, expect, it } from 'vitest'
import { editable, stepOf, taxIdOk } from '../app/utils/org'

const base = { legal_name: 'a', display_name: 'a', org_type: 'association', tax_id: '04595257', registration_no: 'x', authority: 'y',
  rep_name: 'r', contact_name: 'c', contact_phone: '0912', public_email: 'a@b.c' }

describe('org utils', () => {
  it('taxIdOk', () => {
    expect(taxIdOk('04595257')).toBe(true)
    expect(taxIdOk('04595258')).toBe(false)
    expect(taxIdOk('0459525')).toBe(false)
    expect(taxIdOk('abcdefgh')).toBe(false)
  })
  it('editable: needs_changes 只開放被點名欄位', () => {
    const o = { verification_status: 'needs_changes', flagged_fields: ['tax_id'] }
    expect(editable(o, 'tax_id')).toBe(true)
    expect(editable(o, 'intro')).toBe(false)
    expect(editable({ verification_status: 'draft' }, 'intro')).toBe(true)
    expect(editable({ verification_status: 'pending' }, 'intro')).toBe(false)
  })
  it('stepOf', () => {
    expect(stepOf({ ...base, tax_id: '' })).toBe(1)
    expect(stepOf({ ...base, rep_name: '' })).toBe(2)
    expect(stepOf(base)).toBe(3)
    const documents = [{ kind: 'registration' }, { kind: 'permit' }]
    expect(stepOf({ ...base, documents })).toBe(3)
    expect(stepOf({ ...base, documents }, [{ id: 'p' }])).toBe(4)
  })
})
