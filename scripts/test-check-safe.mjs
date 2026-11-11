import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { checkSafe } from './check-safe.mjs'

function check(source, relative = 'native-dock/src/main.rs') {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'lumen-safety-test-'))
  try {
    fs.mkdirSync(path.join(root, 'native-dock/src'), { recursive: true })
    fs.mkdirSync(path.join(root, 'tools'))
    for (const file of ['tools/icon_extract.py', 'native-dock/Cargo.toml', 'run_native_dock.py', 'run_native_clock.py']) {
      fs.writeFileSync(path.join(root, file), '')
    }
    fs.mkdirSync(path.dirname(path.join(root, relative)), { recursive: true })
    fs.writeFileSync(path.join(root, relative), source)
    return checkSafe(root)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}
test('discovered source packages receive the same safety checks', () => {
  for (const file of ['widgets/sample/native/mod.rs', 'widgets/sample/adapter/main.ts']) {
    assert.equal(check('SendInput()', file).length, 1)
    assert.equal(check('powershell.exe', file).length, 1)
  }
  assert.equal(check('NetUserEnum()', 'widgets/sample/native/mod.rs').length, 1)
})
test('native source is accepted when it contains no prohibited APIs', () => {
  assert.deepEqual(check('fn main() { println!("Local dock"); }'), [])
})
test('optional portable Python entry points receive safety checks', () => {
  for (const file of ['scripts/portable_python/launch_lumen.py','scripts/portable_python/Launch Lumen.pyw']) {
    assert.equal(check('SendInput()',file).length,1)
    assert.equal(check('powershell.exe',file).length,1)
  }
})
for (const api of ['SendInput', 'GetAsyncKeyState', 'PrintWindow', 'NetUserEnum', 'powershell.exe']) {
  test(`native source guard rejects ${api}`, () => {
    const result = check(`extern "system" { fn ${api}(); }`)
    assert.equal(result.length, 1)
    assert.match(result[0], /native-dock[\\/]src[\\/]main.rs/)
  })
}
