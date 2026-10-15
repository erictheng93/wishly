import { describe, expect, it } from 'vitest'
import { evaluateAudit } from '../scripts/audit-dependencies.mjs'

const bracesUrl = 'https://github.com/advisories/GHSA-vfj7-8cjw-p6xm'
const forgeUrl = 'https://github.com/advisories/GHSA-86w9-cpqp-85rv'
const advisory = (name: string, url: string, range: string) => ({
  name, dependency: name, severity: 'high', url, range,
})
const vulnerability = (name: string, via: any[], severity = 'high') => ({
  name, severity, via, nodes: [`node_modules/${name}`],
})
const report = (vulnerabilities: Record<string, any>) => ({
  auditReportVersion: 2,
  vulnerabilities,
  metadata: { vulnerabilities: {
    high: Object.values(vulnerabilities).filter(v => v.severity === 'high').length,
    critical: Object.values(vulnerabilities).filter(v => v.severity === 'critical').length,
  } },
})

// The same eleven entries and cyclic Nuxt dependency paths as the failing CI artifact.
const knownReport = () => report(Object.fromEntries([
  ['braces', [advisory('braces', bracesUrl, '<=3.0.3')]],
  ['node-forge', [advisory('node-forge', forgeUrl, '<=1.4.0')]],
  ['micromatch', ['braces']], ['fast-glob', ['micromatch']],
  ['globby', ['fast-glob', 'micromatch']], ['listhen', ['node-forge']],
  ['nitropack', ['listhen']], ['@nuxt/nitro-server', ['nitropack', 'nuxt']],
  ['@nuxt/vite-builder', ['nuxt']], ['@nuxt/vite-server', ['nuxt']],
  ['nuxt', ['@nuxt/nitro-server', '@nuxt/vite-builder', '@nuxt/vite-server']],
].map(([name, via]) => [name, vulnerability(name as string, via as any[])])))

describe('audit gate for verified local backports', () => {
  it('accepts only the two backported advisories through all eleven cyclic dependency entries', () => {
    expect(evaluateAudit(knownReport(), 1)).toEqual([forgeUrl, bracesUrl])
  })

  it('accepts a clean audit and preserves the existing high severity threshold', () => {
    expect(evaluateAudit(report({}), 0)).toEqual([])
    expect(evaluateAudit(report({ example: vulnerability('example', [
      { ...advisory('example', 'https://github.com/advisories/GHSA-new-low-risk', '*'), severity: 'moderate' },
    ], 'moderate') }), 0)).toEqual([])
  })

  it.each(['high', 'critical'])('rejects a new %s advisory even when known backports are present', severity => {
    const current = knownReport()
    current.vulnerabilities.example = vulnerability('example', [
      { ...advisory('example', 'https://github.com/advisories/GHSA-new-advisory', '*'), severity },
    ], severity)
    expect(() => evaluateAudit(report(current.vulnerabilities), 1)).toThrow()
  })

  it.each([
    ['new advisory in an already patched package', (r: any) => { r.vulnerabilities.braces.via.push(advisory('braces', 'https://github.com/advisories/GHSA-new-advisory', '*')) }],
    ['changed affected range', (r: any) => { r.vulnerabilities.braces.via[0].range = '<=3.0.4' }],
    ['raised advisory severity', (r: any) => { r.vulnerabilities.braces.via[0].severity = 'critical' }],
    ['mismatched advisory package', (r: any) => { r.vulnerabilities.braces.via[0].dependency = 'other-package' }],
    ['additional unpatched nested copy', (r: any) => { r.vulnerabilities.braces.nodes.push('node_modules/example/node_modules/braces') }],
    ['missing dependency link', (r: any) => { r.vulnerabilities.nuxt.via.push('missing-package') }],
    ['cycle without a backported root', (r: any) => { r.vulnerabilities.nuxt.via = ['nuxt'] }],
    ['missing advisory evidence', (r: any) => { r.vulnerabilities.braces.via = [] }],
    ['malformed via entry', (r: any) => { r.vulnerabilities.braces.via = [null] }],
    ['mismatched severity counts', (r: any) => { r.metadata.vulnerabilities.high = 0 }],
    ['registry error', (r: any) => { r.error = { code: 'ENOAUDIT', message: 'registry unavailable' } }],
    ['unsupported report format', (r: any) => { r.auditReportVersion = 3 }],
  ])('fails closed on %s', (_name, change) => {
    const current = knownReport()
    change(current)
    expect(() => evaluateAudit(current, 1)).toThrow()
  })

  it.each([2, null])('rejects an unexpected npm exit status: %s', status => {
    expect(() => evaluateAudit(knownReport(), status)).toThrow()
  })

  it('does not treat a failed scanner with an empty report as a clean scan', () => {
    expect(() => evaluateAudit(report({}), 1)).toThrow()
    expect(() => evaluateAudit({}, 1)).toThrow()
  })
})
