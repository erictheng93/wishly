import { describe, expect, it } from 'vitest'
import { editable, nextSteps, permitStepError, stepOf, taxIdOk } from '../app/utils/org'

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

describe('申請第 3 步與接下來', () => {
  const doc = { documents: [{ kind: 'registration' }] }
  it('permitStepError', () => {
    expect(permitStepError(doc, [{}], false)).toBeNull()
    expect(permitStepError(doc, [], false)).toBe('org.apply.docsRequired')
    expect(permitStepError({ documents: [{ kind: 'permit' }] }, [{}], false)).toBe('org.apply.docsRequired')
    expect(permitStepError(null, [], false)).toBe('org.apply.docsRequired')
    expect(permitStepError(doc, [{}], true)).toBe('org.apply.permitPending')
  })
  it('nextSteps', () => {
    const done = (e: unknown[], s: any[]) => nextSteps(e, s).map(x => x.done)
    expect(done([], [])).toEqual([false, false, false])
    expect(done([{}], [{ status: 'draft', site_status: 'active' }])).toEqual([true, true, false])
    expect(done([{}], [{ status: 'active', site_status: 'paused' }])).toEqual([true, true, false])
    expect(done([{}], [{ status: 'active', site_status: 'active' }])).toEqual([true, true, true])
  })
})
