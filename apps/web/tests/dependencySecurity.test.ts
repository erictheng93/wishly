import { generateKeyPairSync } from 'node:crypto'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRequire } from 'node:module'
import { describe, expect, it } from 'vitest'

const require = createRequire(import.meta.url)
// 對 listhen 實際使用的 node-forge 副本驗證（上游 1.4.1 起已修補 GHSA-86w9-cpqp-85rv，見 patches/README.md）
const forge = createRequire(require.resolve('listhen'))('node-forge')

describe('GHSA-86w9-cpqp-85rv', () => {
  const keys = generateKeyPairSync('rsa', {
    modulusLength: 2048,
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

// package.json 的 override 鍵是 archiver@8.0.0：archiver 一升版就不再套用，這裡會先失敗。
describe('archiver → readdir-glob → minimatch override', () => {
  it('readdir-glob resolves a patched minimatch', () => {
    const entry = createRequire(require.resolve('readdir-glob')).resolve('minimatch')
    const root = entry.slice(0, entry.lastIndexOf('/minimatch/') + '/minimatch/'.length)
    const { version } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
    expect(Number(version.split('.')[0])).toBeGreaterThanOrEqual(11)
  })

  it('still zips by glob with the overridden readdir-glob', async () => {
    const { ZipArchive } = await import('archiver')
    const dir = mkdtempSync(join(tmpdir(), 'wishly-zip-'))
    try {
      mkdirSync(join(dir, 'sub'))
      for (const f of ['a.txt', 'b.md', 'sub/c.txt']) writeFileSync(join(dir, f), f)
      const zip = new ZipArchive()
      const chunks: Buffer[] = []
      zip.on('data', (c: Buffer) => chunks.push(c))
      const done = new Promise((resolve, reject) => zip.on('end', resolve).on('error', reject))
      zip.glob('**/*.txt', { cwd: dir })
      await zip.finalize()
      await done
      const out = Buffer.concat(chunks).toString('latin1') // 檔名在 zip 標頭中未壓縮
      expect(out).toContain('a.txt')
      expect(out).toContain('sub/c.txt')
      expect(out).not.toContain('b.md')
    }
    finally { rmSync(dir, { recursive: true, force: true }) }
  })
})
