import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import { EventEmitter } from 'node:events'
import ts from 'typescript'
import path from 'node:path'

function adapter() {
  const calls = [], output = [], input = new EventEmitter()
  input.close = () => input.emit('close')
  let instance, changed, factory
  class Player {
    ready = true
    constructor(directory, callback, host) { instance = this; changed = callback; factory = host; calls.push(['create', directory]) }
    async open(url) { calls.push(['browse', url]) }
    async play(id) { calls.push(['select', id]) }
    async command(action, value) { calls.push([action, value]) }
    async instanceInfo() { return { instanceId: 'same-page', pageCount: 1 } }
    stop() { calls.push(['stop']) }
    hide() { calls.push(['hide']) }
  }
  const process = Object.assign(new EventEmitter(), { cwd: () => 'D:/Lumen', argv: ['node', 'adapter', '1234'], stdin: {}, stdout: { write: line => output.push(JSON.parse(line)) }, exit: () => calls.push(['exit']) })
  const realRequire = {
    'node:fs': { mkdirSync() {}, existsSync: () => true },
    'node:path': path,
    'node:readline': { createInterface: () => input },
    'node:crypto': { createHash: () => ({ update() { return this }, digest: () => 'a'.repeat(64) }) },
    './musicPlayer': { MusicPlayer: Player },
    './musicPlayerWindow': { createMusicWindowHost: pid => { calls.push(['host', pid]); return { show: () => calls.push(['show-browser', pid]), hide() {}, destroy() {} } } },
    koffi: { load: () => ({ func: name => (...args) => calls.push([name.includes('ShowWindow') ? 'show' : 'foreground', ...args]) }) }
  }
  const source = ts.transpileModule(fs.readFileSync(new URL('../widgets/music/adapter/nativeMusicBridge.ts', import.meta.url), 'utf8'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, esModuleInterop: true } }).outputText
  vm.runInNewContext(source, { require: name => { if (!(name in realRequire)) throw Error(name); return realRequire[name] }, exports: {}, process, Buffer, URL, AbortSignal, setTimeout: () => ({ unref() {} }) })
  return { calls, output, input, instance, host: (...args) => factory(...args), changed: (...args) => changed(...args), send: message => input.emit('line', JSON.stringify(message)), settle: () => new Promise(resolve => setImmediate(resolve)) }
}

test('native adapter routes controls through one player without embedding or resizing a second frame', async () => {
  const a = adapter()
  a.host(42, { top: 30, side: 4 })
  a.send({ action: 'browse' }); a.send({ action: 'browse' }); a.send({ action: 'seek', value: 125 }); a.send({ action: 'select', videoId: 'abcdefghijk' }); a.send({ action: 'layout' }); a.send({ action: 'info' })
  await a.settle()
  assert.equal(a.calls.filter(c => c[0] === 'create').length, 1)
  assert.equal(a.calls.filter(c => c[0] === 'browse').length, 2)
  assert.equal(a.calls.filter(c => c[0] === 'host').length, 1)
  assert.equal(a.calls.filter(c => c[0] === 'show' || c[0] === 'mount').length, 0)
  assert.ok(a.calls.some(c => c[0] === 'seek' && c[1] === 125))
  assert.ok(a.calls.some(c => c[0] === 'select' && c[1] === 'abcdefghijk'))
  assert.ok(a.output.some(e => e.type === 'info' && e.instanceId === 'same-page' && e.pageCount === 1))
})
test('native adapter rejects unknown controls, bounds input, and closes its player once on EOF', async () => {
  const a = adapter()
  a.input.emit('line', '{broken'); a.input.emit('line', 'x'.repeat(5000)); a.send({ action: 'execute-script' })
  await a.settle()
  assert.equal(a.output.filter(e => e.type === 'error').length, 1)
  assert.equal(a.output.find(e => e.type === 'error').error, 'Unknown native music action')
  a.input.emit('close'); a.input.emit('close')
  assert.equal(a.calls.filter(c => c[0] === 'stop').length, 1)
  a.send({ action: 'browse' })
  await a.settle()
  assert.equal(a.calls.filter(c => c[0] === 'browse').length, 0)
})

test('library navigation reuses the existing music player', async () => {
  const a = adapter()
  a.send({ action: 'library' })
  await a.settle()
  assert.equal(a.calls.filter(c => c[0] === 'create').length, 1)
  assert.ok(a.calls.some(c => c[0] === 'browse' && c[1] === 'https://music.youtube.com/library'))
})
test('artwork prioritizes the visible Coverflow neighbors for a track late in the queue', () => {
  const a = adapter()
  a.changed({ art: 'https://i.ytimg.com/current.jpg', videoId: 'track-25', queue: Array.from({ length: 30 }, (_, i) => ({ videoId: `track-${i}`, selected: i === 25, thumb: `https://i.ytimg.com/track-${i}.jpg` })) })
  assert.deepEqual(a.output.filter(e => e.type === 'art').slice(0, 6).map(e => e.url), [
    'https://i.ytimg.com/current.jpg', ...[23, 24, 25, 26, 27].map(i => `https://i.ytimg.com/track-${i}.jpg`)
  ])
})
