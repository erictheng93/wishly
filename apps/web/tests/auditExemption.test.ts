import { describe, expect, it } from 'vitest'
// @ts-expect-error 純 JS 腳本，無型別宣告
import { auditExemption, EXEMPTIONS } from '../scripts/audit-dependencies.mjs'

const GHSA = EXEMPTIONS[0].ghsa
const adv = (ghsa: string) => ({ name: 'x', severity: 'high', url: `https://github.com/advisories/${ghsa}` })
const vuln = (via: any[], severity = 'high') => ({ severity, via })
const today = '2026-11-18'

describe('auditExemption', () => {
  it('只有該公告與其遞移相依 -> 豁免', () => {
    const r = auditExemption({ vulnerabilities: {
      picomatch: vuln([adv(GHSA)]), vite: vuln(['picomatch']), nuxt: vuln(['vite']),
    } }, today)
    expect(r.exempt).toBe(true)
    expect(r.packages).toBe(3)
  })
  it('另有不相干的 high -> 不豁免', () => {
    expect(auditExemption({ vulnerabilities: {
      picomatch: vuln([adv(GHSA)]), other: vuln([adv('GHSA-aaaa-bbbb-cccc')]),
    } }, today).exempt).toBe(false)
  })
  it('同套件同時有別的公告 -> 不豁免', () => {
    expect(auditExemption({ vulnerabilities: {
      picomatch: vuln([adv(GHSA), adv('GHSA-aaaa-bbbb-cccc')]), vite: vuln(['picomatch']),
    } }, today).exempt).toBe(false)
  })
  it('以公告編號而非套件名比對', () => {
    expect(auditExemption({ vulnerabilities: {
      picomatch: vuln([adv('GHSA-aaaa-bbbb-cccc')]),
    } }, today).exempt).toBe(false)
  })
  it('過期後不豁免（到期日當天仍有效）', () => {
    const v = { vulnerabilities: { picomatch: vuln([adv(GHSA)]) } }
    expect(auditExemption(v, EXEMPTIONS[0].expires).exempt).toBe(true)
    expect(auditExemption(v, '2026-12-19').exempt).toBe(false)
  })
  it('via 循環不會當掉', () => {
    expect(auditExemption({ vulnerabilities: {
      picomatch: vuln([adv(GHSA)]), a: vuln(['b', 'picomatch']), b: vuln(['a']),
    } }, today).exempt).toBe(true)
    expect(auditExemption({ vulnerabilities: { a: vuln(['b']), b: vuln(['a']) } }, today).exempt).toBe(false)
  })
  it('high 套件同時指向 moderate 套件仍可豁免', () => {
    expect(auditExemption({ vulnerabilities: {
      picomatch: vuln([adv(GHSA)]), nitropack: vuln(['picomatch', 'archiver']),
      archiver: vuln(['minimatch'], 'moderate'), minimatch: vuln([adv('GHSA-aaaa-bbbb-cccc')], 'moderate'),
    } }, today).exempt).toBe(true)
  })
  it('moderate 不影響結果；無 high/critical 或無法解析時不豁免', () => {
    expect(auditExemption({ vulnerabilities: {
      picomatch: vuln([adv(GHSA)]), minimatch: vuln([adv('GHSA-aaaa-bbbb-cccc')], 'moderate'),
    } }, today).exempt).toBe(true)
    expect(auditExemption({ vulnerabilities: { m: vuln([adv('GHSA-aaaa-bbbb-cccc')], 'moderate') } }, today).exempt).toBe(false)
    expect(auditExemption({ error: { code: 'ENOAUDIT' } }, today).exempt).toBe(false)
    expect(auditExemption(null, today).exempt).toBe(false)
  })
})
