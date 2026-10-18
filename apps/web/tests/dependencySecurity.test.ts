import { generateKeyPairSync } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { describe, expect, it } from 'vitest'

const require = createRequire(import.meta.url)
// Exercise the actual transitive copies used by globby/fast-glob and listhen (package.json overrides → wishly forks).
const braces = createRequire(require.resolve('micromatch'))('braces')
const forge = createRequire(require.resolve('listhen'))('node-forge')

describe('GHSA-vfj7-8cjw-p6xm', () => {
  const deepPatterns = [
    '{'.repeat(4500) + 'a' + '}'.repeat(4500),
    '('.repeat(4500) + 'a' + ')'.repeat(4500),
    '({'.repeat(2250) + 'a' + '})'.repeat(2250),
    '{'.repeat(4500) + 'a',
  ]

  it.each(['parse', 'compile', 'expand', 'stringify'])('%s rejects excessive pattern depth before stack exhaustion', method => {
    for (const pattern of deepPatterns) {
      expect(() => braces[method](pattern)).toThrow(SyntaxError)
    }
  })

  it.each(['compile', 'expand', 'stringify'])('%s also bounds caller-supplied ASTs and cycles', method => {
    const ast: any = { type: 'root', nodes: [] }
    let node = ast
    for (let i = 0; i < 4500; i++) {
      const child = { type: 'paren', nodes: [], parent: node }
      node.nodes.push(child)
      node = child
    }
    node.nodes.push({ type: 'text', value: 'a' })
    expect(() => braces[method](ast)).toThrow(SyntaxError)
    const cyclic: any = { type: 'root', nodes: [] }
    cyclic.nodes.push(cyclic)
    expect(() => braces[method](cyclic)).toThrow(SyntaxError)
  })

  it('preserves ordinary expansion, nesting, and literal delimiters', () => {
    expect(braces.expand('src/{app,test}/{1..2}.ts')).toEqual([
      'src/app/1.ts', 'src/app/2.ts', 'src/test/1.ts', 'src/test/2.ts',
    ])
    const nested = '{'.repeat(32) + 'a' + '}'.repeat(32)
    expect(braces.stringify(nested)).toBe(nested)
    for (const literal of [
      '['.repeat(4500) + 'a' + ']'.repeat(4500),
      '"' + '{'.repeat(4500) + '"',
      '\\{'.repeat(4500),
    ]) {
      expect(() => braces.compile(literal)).not.toThrow()
      expect(() => braces.expand(literal)).not.toThrow()
      expect(() => braces.stringify(literal)).not.toThrow()
    }
  })

  it('bounds expand ancestry walks on a caller-supplied parent cycle', () => {
    const result = spawnSync(process.execPath, ['-e', `
      const braces = require(${JSON.stringify(require.resolve('braces'))});
      const ast = { type: 'paren', nodes: [] };
      ast.parent = ast;
      try { braces.expand(ast); process.exitCode = 1; }
      catch (error) { if (!(error instanceof SyntaxError)) throw error; }
    `], { timeout: 2000, encoding: 'utf8' })
    expect(result.status, result.error?.message ?? result.stderr).toBe(0)
  })
})

describe('GHSA-86w9-cpqp-85rv', () => {
  const keys = generateKeyPairSync('rsa', {
    modulusLength: 1024,
    publicExponent: 3,
    publicKeyEncoding: { type: 'spki', format: 'pem' },
    privateKeyEncoding: { type: 'pkcs8', format: 'pem' },
  })
  const privateKey = forge.pki.privateKeyFromPem(keys.privateKey)
  const publicKey = forge.pki.publicKeyFromPem(keys.publicKey)
  const { asn1 } = forge
  const value = (type: number, content: any) => asn1.create(asn1.Class.UNIVERSAL, type, type === asn1.Type.SEQUENCE, content)
  const digest = forge.md.sha256.create().update('dependency security regression').digest().getBytes()
  const oid = () => value(asn1.Type.OID, asn1.oidToDer(forge.pki.oids.sha256).getBytes())
  const nil = (content = '') => value(asn1.Type.NULL, content)
  const garbage = () => value(asn1.Type.OCTETSTRING, 'unconsumed attacker bytes')
  const signature = (algorithm: any[]) => privateKey.sign(asn1.toDer(value(asn1.Type.SEQUENCE, [
    value(asn1.Type.SEQUENCE, algorithm), value(asn1.Type.OCTETSTRING, digest),
  ])).getBytes(), 'NONE')

  it.each([
    ['extra element', () => [oid(), nil(), garbage()]],
    ['garbage replacing optional NULL', () => [oid(), garbage()]],
    ['duplicate NULL', () => [oid(), nil(), nil()]],
    ['nonempty NULL', () => [oid(), nil('unconsumed attacker bytes')]],
  ])('rejects %s inside DigestAlgorithm', (_name, algorithm) => {
    const signed = signature(algorithm())
    expect(() => publicKey.verify(digest, signed)).toThrow(/DigestInfo/)
  })

  it('preserves SHA256 with optional NULL and ordinary signatures', () => {
    for (const algorithm of [[oid()], [oid(), nil()]]) {
      expect(publicKey.verify(digest, signature(algorithm))).toBe(true)
    }
    const md = forge.md.sha256.create().update('ordinary signed message')
    const signed = privateKey.sign(md)
    expect(publicKey.verify(md.digest().getBytes(), signed)).toBe(true)
    expect(publicKey.verify(digest, signed)).toBe(false)
  })

  it('preserves certificate signing and verification used by development HTTPS', () => {
    const certificate = forge.pki.createCertificate()
    certificate.publicKey = publicKey
    certificate.serialNumber = '01'
    certificate.validity.notBefore = new Date('2026-01-01')
    certificate.validity.notAfter = new Date('2027-01-01')
    const attributes = [{ name: 'commonName', value: 'localhost' }]
    certificate.setSubject(attributes)
    certificate.setIssuer(attributes)
    certificate.sign(privateKey, forge.md.sha256.create())
    const parsed = forge.pki.certificateFromPem(forge.pki.certificateToPem(certificate))
    expect(parsed.verify(parsed)).toBe(true)
  })
})
