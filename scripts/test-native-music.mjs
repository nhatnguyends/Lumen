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
    async signIn() { calls.push(['signin']) }
    async signOut() { calls.push(['signout']) }
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

test('sign-in opens a normal browser only after closing managed playback and prevents concurrent profile use', async () => {
  const exports = {}, calls = [], child = new EventEmitter()
  const source = ts.transpileModule(fs.readFileSync(new URL('../widgets/music/adapter/musicPlayer.ts', import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, esModuleInterop: true }
  }).outputText
  const modules = {
    'node:fs': { readFileSync() { throw Error('first run') }, existsSync: () => true, mkdirSync() {}, writeFileSync() {}, renameSync() {} },
    'node:path': path,
    'node:child_process': { spawn(exe, args) { calls.push(['spawn', exe, args]); setImmediate(() => child.emit('spawn')); return child } },
    'node:crypto': {}, 'node:http': {}, './musicPipe': {}, './musicPage': {}, './musicPlayerWindow': {}
  }
  vm.runInNewContext(source, { exports, require: name => modules[name], process: { env: { ProgramFiles: 'C:/Apps' } }, URL, setInterval, clearInterval, setTimeout })
  const player = new exports.MusicPlayer('fixture')
  player.pipe = { closed: false, send: async () => { calls.push(['Browser.close']) }, close() {} }
  let releaseProfile
  player.browserExit = new Promise(resolve => { releaseProfile = resolve })
  const signingIn = player.signIn()
  await new Promise(resolve => setImmediate(resolve))
  assert.ok(!calls.some(c => c[0] === 'spawn'), 'profile must be released before sign-in launch')
  releaseProfile()
  await signingIn
  assert.equal(calls[0][0], 'Browser.close')
  const args = calls.find(c => c[0] === 'spawn')[2]
  assert.ok(args.includes(`--user-data-dir=${path.join('fixture', 'music-player', 'chrome')}`))
  assert.ok(args.includes('--profile-directory=LumenMusic'))
  assert.ok(args.includes('--new-window'))
  assert.ok(args.includes('https://music.youtube.com/'))
  assert.ok(!args.some(arg => /remote-debug|automation|--app=/.test(arg)))
  await assert.rejects(player.ensure(), /Finish signing in/)
  await assert.rejects(player.signIn(), /already open/)
  child.emit('exit', 0)
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(player.signingIn, null)
  player.stopped = false
  player.current = { title: 'A', videoId: 'aaaaaaaaaaa', playlistId: 'playlist', signedIn: true,
    queue: ['aaaaaaaaaaa', 'bbbbbbbbbbb', 'ccccccccccc'].map(videoId => ({ videoId })) }
  player.evaluate = async () => ({ title: 'B', videoId: 'bbbbbbbbbbb', playlistId: 'playlist', available: true,
    signedIn: null, position: 0, queue: ['bbbbbbbbbbb', 'ccccccccccc', 'ddddddddddd'].map(videoId => ({ videoId })) })
  await player.publish()
  assert.deepEqual(Array.from(player.current.queue, t => t.videoId), ['bbbbbbbbbbb', 'ccccccccccc', 'ddddddddddd'])
  assert.equal(player.current.signedIn, true, 'loading navbar cannot revoke a confirmed login')
  assert.equal(player.current.authKnown, false)
  player.pipe = { closed: false, send: async (method, params, session) => { calls.push([method, params, session]) } }
  player.playbackSession = 'owned-session'
  player.current = { signedIn: true, playing: true, available: true }
  await player.signOut()
  assert.ok(calls.some(c => c[0] === 'Network.clearBrowserCookies' && c[2] === 'owned-session'))
  assert.ok(calls.some(c => c[0] === 'Page.navigate' && c[1].url === 'https://music.youtube.com/'))
  assert.equal(player.current.signedIn, false)
  assert.equal(player.current.authKnown, true)
  assert.equal(player.current.playing, false)
  const a = adapter()
  a.send({ action: 'signin' }); await a.settle()
  assert.ok(a.calls.some(c => c[0] === 'signin'))
  a.send({ action: 'signout' }); await a.settle()
  assert.ok(a.calls.some(c => c[0] === 'signout'))
})

test('page state uses complete playlist data and current player ID instead of stale URL or metadata', () => {
  const exports = {}
  vm.runInNewContext(ts.transpileModule(fs.readFileSync(new URL('../widgets/music/adapter/musicPage.ts', import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 }
  }).outputText, { exports })
  const contents = ['aaaaaaaaaaa', 'bbbbbbbbbbb', 'ccccccccccc'].map((videoId, i) => ({ playlistPanelVideoRenderer: {
    videoId, title: { simpleText: `Track ${i}` }, selected: i === 0,
    thumbnail: { thumbnails: [{ url: `https://i.ytimg.com/${videoId}.jpg` }] }
  } }))
  const panel = { data: { contents }, querySelectorAll: () => [] }
  const document = {
    getElementById: () => ({ getVideoData: () => ({ video_id: 'bbbbbbbbbbb' }) }),
    querySelector(selector) {
      if (selector === 'ytmusic-player-bar') return { querySelector: () => null }
      if (selector === 'video') return { readyState: 4, duration: 100, currentTime: 0 }
      if (selector.includes('ytmusic-player-queue')) return panel
      return null
    }
  }
  const state = vm.runInNewContext(exports.MUSIC_PAGE_STATE, {
    document, window: { ytcfg: { get: () => true } }, navigator: { mediaSession: { metadata: { title: 'Stale A' } } },
    location: { origin: 'https://music.youtube.com', href: 'https://music.youtube.com/watch?v=aaaaaaaaaaa' }, URL
  })
  assert.equal(state.videoId, 'bbbbbbbbbbb')
  assert.equal(state.title, 'Track 1')
  assert.equal(state.art, 'https://i.ytimg.com/bbbbbbbbbbb.jpg')
  assert.equal(state.signedIn, true)
  assert.deepEqual(Array.from(state.queue, t => t.selected), [false, true, false])
})
test('artwork prioritizes the visible Coverflow neighbors for a track late in the queue', () => {
  const a = adapter()
  a.changed({ art: 'https://i.ytimg.com/current.jpg', videoId: 'track-25', queue: Array.from({ length: 30 }, (_, i) => ({ videoId: `track-${i}`, selected: i === 25, thumb: `https://i.ytimg.com/track-${i}.jpg` })) })
  assert.deepEqual(a.output.filter(e => e.type === 'art').slice(0, 6).map(e => e.url), [
    'https://i.ytimg.com/current.jpg', ...[23, 24, 25, 26, 27].map(i => `https://i.ytimg.com/track-${i}.jpg`)
  ])
})
test('startup Coverflow preloads true next covers without treating playlist end as previous', () => {
  const a = adapter()
  a.changed({ art: 'https://i.ytimg.com/current.jpg', videoId: 'track-0', queue: Array.from({ length: 30 }, (_, i) => ({ videoId: `track-${i}`, selected: i === 0, thumb: `https://i.ytimg.com/track-${i}.jpg` })) })
  assert.deepEqual(a.output.filter(e => e.type === 'art').slice(1, 4).map(e => e.url),
    [0, 1, 2].map(i => `https://i.ytimg.com/track-${i}.jpg`))
})
test('page state unwraps playlistPanelVideoWrapperRenderer without duplicating counterparts', () => {
  const exports = {}
  vm.runInNewContext(ts.transpileModule(fs.readFileSync(new URL('../widgets/music/adapter/musicPage.ts', import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 }
  }).outputText, { exports })
  const contents = [
    {
      playlistPanelVideoWrapperRenderer: {
        primaryRenderer: { playlistPanelVideoRenderer: {
          videoId: 'JBbSlADtG-g', title: { simpleText: 'UNETHICAL' }, selected: true,
          thumbnail: { thumbnails: [{ url: 'https://yt3.googleusercontent.com/song.jpg' }] }
        } },
        counterpart: [{ counterpartRenderer: { playlistPanelVideoRenderer: {
          videoId: 'I1zqCX3Dvxo', title: { simpleText: 'UNETHICAL' }, selected: false,
          thumbnail: { thumbnails: [{ url: 'https://i.ytimg.com/vi/I1zqCX3Dvxo/hq720.jpg' }] }
        } } }]
      }
    },
    {
      playlistPanelVideoWrapperRenderer: {
        primaryRenderer: { playlistPanelVideoRenderer: {
          videoId: 'zPEJDNZWKeU', title: { simpleText: 'Follow You' }, selected: false,
          thumbnail: { thumbnails: [{ url: 'https://yt3.googleusercontent.com/follow.jpg' }] }
        } },
        counterpart: [{ counterpartRenderer: { playlistPanelVideoRenderer: {
          videoId: 'k3zimSRKqNw', title: { simpleText: 'Follow You (Official Music Video)' }, selected: false,
          thumbnail: { thumbnails: [{ url: 'https://i.ytimg.com/vi/k3zimSRKqNw/hq720.jpg' }] }
        } } }]
      }
    }
  ]
  const run = (videoId) => vm.runInNewContext(exports.MUSIC_PAGE_STATE, {
    document: {
      getElementById: () => ({ getVideoData: () => ({ video_id: videoId }) }),
      querySelector(selector) {
        if (selector === 'ytmusic-player-bar') return { querySelector: () => null }
        if (selector === 'video') return { readyState: 4, duration: 100, currentTime: 0 }
        if (selector.includes('ytmusic-player-queue')) return { data: { contents }, querySelectorAll: () => [] }
        return null
      }
    },
    window: { ytcfg: { get: () => true } },
    navigator: { mediaSession: { metadata: {} } },
    location: { origin: 'https://music.youtube.com', href: `https://music.youtube.com/watch?v=${videoId}` },
    URL
  })
  const songState = run('JBbSlADtG-g')
  assert.deepEqual(Array.from(songState.queue, t => t.videoId), ['JBbSlADtG-g', 'zPEJDNZWKeU'])
  assert.equal(songState.art, 'https://yt3.googleusercontent.com/song.jpg')
  const videoState = run('I1zqCX3Dvxo')
  assert.deepEqual(Array.from(videoState.queue, t => t.videoId), ['I1zqCX3Dvxo', 'zPEJDNZWKeU'])
  assert.equal(videoState.art, 'https://i.ytimg.com/vi/I1zqCX3Dvxo/hq720.jpg')
})
test('page state DOM fallback ignores counterpart queue items', () => {
  const exports = {}
  vm.runInNewContext(ts.transpileModule(fs.readFileSync(new URL('../widgets/music/adapter/musicPage.ts', import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 }
  }).outputText, { exports })
  const makeRow = (videoId, title, thumb, isCounterpart = false) => ({
    data: { playlistPanelVideoRenderer: { videoId, title: { simpleText: title }, thumbnail: { thumbnails: [{ url: thumb }] } } },
    hasAttribute: () => false,
    querySelector: () => null,
    closest: (sel) => isCounterpart && sel.includes('counterpart') ? {} : null
  })
  const rows = [
    makeRow('JBbSlADtG-g', 'UNETHICAL', 'https://yt3.googleusercontent.com/song.jpg', false),
    makeRow('I1zqCX3Dvxo', 'UNETHICAL', 'https://i.ytimg.com/vi/I1zqCX3Dvxo/hq720.jpg', true),
    makeRow('zPEJDNZWKeU', 'Follow You', 'https://yt3.googleusercontent.com/follow.jpg', false)
  ]
  const state = vm.runInNewContext(exports.MUSIC_PAGE_STATE, {
    document: {
      getElementById: () => ({ getVideoData: () => ({ video_id: 'JBbSlADtG-g' }) }),
      querySelector(selector) {
        if (selector === 'ytmusic-player-bar') return { querySelector: () => null }
        if (selector === 'video') return { readyState: 4, duration: 100, currentTime: 0 }
        if (selector.includes('ytmusic-player-queue')) return { querySelectorAll: () => rows }
        return null
      }
    },
    window: { ytcfg: { get: () => true } },
    navigator: { mediaSession: { metadata: {} } },
    location: { origin: 'https://music.youtube.com', href: 'https://music.youtube.com/watch?v=JBbSlADtG-g' },
    URL
  })
  assert.deepEqual(Array.from(state.queue, t => t.videoId), ['JBbSlADtG-g', 'zPEJDNZWKeU'])
})
