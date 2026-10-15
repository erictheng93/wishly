import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

// Temporary backports for issue #7; remove only after upstream fixes pass the regression tests.
const root = new URL('../', import.meta.url)
const files = {
  'braces/lib/parse.js': [
    'e572166565f15fa6ad9865ae49d678218e32aabfd1b3720f6d0d43d39800d310',
    'f2ba032613e8449d7faa8dcbed8369f030e32899969daca39d01d5212961f2a5',
  ],
  'braces/lib/compile.js': [
    'dc98f22eee3d511785d92a00758d5f0d48efed5f5813bdecc2de430c529b5c9f',
    '273f1eb55ebe0f99853fef6879360d9a50c5e2347b2ee29a63c5ccb18f6c071c',
  ],
  'braces/lib/expand.js': [
    '41ccc196ebfa7b7781a634e721eb744e4e7bcb54cba427a7e3d6806a1b9e58f7',
    '20a59cc81a7afb659e54b5b23b5a4eeb376bd8b69247b5baa5a3db7196293c3f',
  ],
  'braces/lib/stringify.js': [
    '379f22d77bfa1478341ccd49c5e4267464aabcbba03558bab332aac23fc6f23a',
    '9fe18e01e6854147b59cc18db88dcedc8bd285db1315f360407a4e59f657e4c4',
  ],
  'node-forge/lib/rsa.js': [
    'fd4740238145ec26470eb3f06a627c72039538ce1307dbdce40521f94dfd0a50',
    '1b68c0216cf74fdc120d964b0b58b7c214bfe3ce9e9927b9fd8472d532fd40ee',
  ],
}

for (const [name, version] of [['braces', '3.0.3'], ['node-forge', '1.4.0']]) {
  const installed = JSON.parse(readFileSync(new URL(`node_modules/${name}/package.json`, root)))
  assert.equal(installed.version, version, `Review ${name} security backport before upgrading`)
}

const digest = path => createHash('sha256').update(readFileSync(new URL(`node_modules/${path}`, root))).digest('hex')
if (!Object.entries(files).every(([path, hashes]) => digest(path) === hashes[1])) {
  for (const [path, hashes] of Object.entries(files)) {
    assert.equal(digest(path), hashes[0], `Unexpected dependency source: ${path}; reinstall or review the backport`)
  }
  execFileSync('git', ['apply', '--no-index', '--whitespace=error', 'patches/dependency-security.patch'], {
    cwd: fileURLToPath(root), stdio: 'inherit',
  })
}
for (const [path, hashes] of Object.entries(files)) {
  assert.equal(digest(path), hashes[1], `Security backport was not applied: ${path}`)
}
console.log('Verified local braces and node-forge security backports')
