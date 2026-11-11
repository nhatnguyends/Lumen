import fs from 'node:fs'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { randomBytes, randomInt } from 'node:crypto'
import { createServer } from 'node:http'
import type { Readable, Writable } from 'node:stream'
import { MusicPipe } from './musicPipe'
import { MUSIC_PAGE_STATE, musicPageCommand, musicPageSelect, type PlayerPageState } from './musicPage'
import { hideMusicBackgroundWindow, type MusicFrameInsets } from './musicPlayerWindow'

export type PlayerBrowser = 'edge' | 'chrome'
export interface PlayerIdentity {
  browser: PlayerBrowser | null
  appId: string
}

export function playerBrowserCandidates(env: NodeJS.ProcessEnv = process.env): { browser: PlayerBrowser; executable: string }[] {
  const roots = [env['ProgramFiles(x86)'], env.ProgramW6432, env.ProgramFiles, env.LOCALAPPDATA].filter(Boolean) as string[]
  return (['chrome', 'edge'] as const).flatMap((browser) => roots.map((root) => ({
    browser,
    executable: path.join(root, ...(browser === 'edge' ? ['Microsoft', 'Edge', 'Application', 'msedge.exe'] : ['Google', 'Chrome', 'Application', 'chrome.exe']))
  })))
}

export function playerArguments(profile: string, url = 'https://music.youtube.com/'): string[] {
  const target = new URL(url)
  if (target.origin !== 'https://music.youtube.com' || target.username || target.password) throw new Error('Invalid YouTube Music player URL')
  return [
    `--user-data-dir=${profile}`, '--profile-directory=LumenMusic',
    '--no-first-run', '--no-default-browser-check', `--app=${target.href}`
  ]
}

export interface MusicWindowHost { show(): void; hide(): void; destroy(): void }

/** Exactly one music page. Showing/hiding its frame never creates a target. */
export class MusicPlayer {
  identity: PlayerIdentity = { browser: null, appId: '' }
  pid = 0
  private launching: Promise<void> | null = null
  private pipe: MusicPipe | null = null
  private playbackId = ''
  private playbackSession = ''
  private host: MusicWindowHost | null = null
  private sessions = new Map<string, string>()
  private timer: NodeJS.Timeout | null = null
  private polling = false
  private changing = false
  private playbackRequest = 0
  private stopped = false
  private closing: Promise<void> | null = null
  private resume = { videoId: '', position: 0 }
  current: PlayerPageState | null = null

  constructor(private readonly directory: string,
    private readonly changed: (state: PlayerPageState | null, error?: string) => void = () => {},
    private readonly createHost?: (pid: number, insets: MusicFrameInsets, onClose: () => void) => MusicWindowHost) {
    try {
      const raw = JSON.parse(fs.readFileSync(path.join(directory, 'music-player.json'), 'utf8'))
      if (raw.browser === 'edge' || raw.browser === 'chrome') this.identity.browser = raw.browser
      if (typeof raw.appId === 'string' && raw.appId.length < 256 && raw.appId.includes('LumenMusic')) this.identity.appId = raw.appId
      if (/^[\w-]{11}$/.test(raw.videoId || '')) this.resume.videoId = raw.videoId
      if (Number.isFinite(raw.position)) this.resume.position = Math.max(0, raw.position)
    } catch { /* first run */ }
  }

  rememberAppId(appId: string): void {
    if (!appId.includes('LumenMusic') || appId === this.identity.appId || appId.length >= 256) return
    this.identity.appId = appId
    this.save()
  }

  private save(): void {
    fs.mkdirSync(this.directory, { recursive: true })
    const file = path.join(this.directory, 'music-player.json')
    fs.writeFileSync(`${file}.tmp`, JSON.stringify({ ...this.identity, ...this.resume }, null, 2))
    fs.renameSync(`${file}.tmp`, file)
  }

  get ready(): boolean { return !!this.pipe && !this.pipe.closed && !!this.playbackSession }
  get starting(): boolean { return this.launching !== null }
  get lastVideoId(): string { return this.resume.videoId }

  async ensure(): Promise<void> {
    if (this.launching) return this.launching
    if (this.ready) return
    if (!this.launching) {
      this.stopped = false
      this.launching = (async () => {
        await this.closing
        if (this.stopped) throw new Error('Music widget closed.')
        await this.launch()
      })().finally(() => { this.launching = null })
    }
    return this.launching
  }

  private async launch(): Promise<void> {
    this.host?.destroy()
    this.host = null
    this.current = null
    this.playbackSession = ''
    this.sessions.clear()
    const candidates = playerBrowserCandidates()
    if (this.identity.browser) candidates.sort((a, b) => Number(b.browser === this.identity.browser) - Number(a.browser === this.identity.browser))
    const found = candidates.find((candidate) => fs.existsSync(candidate.executable))
    if (!found) throw new Error('Install Microsoft Edge or Google Chrome to open the music player.')
    // Keep browser profiles separate, and keep both separate from normal browsing.
    const profile = path.join(this.directory, 'music-player', found.browser)
    const title = `Lumen Music Background ${randomBytes(8).toString('hex')}`
    const bootstrapPath = `/lumen-background/${randomBytes(12).toString('hex')}`
    const bootstrap = createServer((request, response) => {
      if (request.url !== bootstrapPath) { response.writeHead(404).end(); return }
      response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store', 'Content-Security-Policy': "default-src 'none'; frame-ancestors 'none'" })
      response.end(`<!doctype html><title>${title}</title>`)
    })
    // Windows may allocate an ephemeral port that browsers prohibit (e.g. 6000).
    for (let attempt = 0; ; attempt++) {
      try {
        await new Promise<void>((resolve, reject) => {
          const ready = () => { bootstrap.off('error', failed); resolve() }
          const failed = (error: Error) => { bootstrap.off('listening', ready); reject(error) }
          bootstrap.once('error', failed)
          bootstrap.once('listening', ready)
          bootstrap.listen(randomInt(20000, 60000), '127.0.0.1')
        })
        break
      } catch (error) {
        if (attempt < 8 && (error as NodeJS.ErrnoException).code === 'EADDRINUSE') continue
        bootstrap.close()
        throw error
      }
    }
    const bootstrapUrl = `http://127.0.0.1:${(bootstrap.address() as { port: number }).port}${bootstrapPath}`
    const args = [...playerArguments(profile).slice(0, -1), '--remote-debugging-pipe', `--app=${bootstrapUrl}`, '--window-position=-32000,-32000',
      '--disable-background-timer-throttling', '--disable-renderer-backgrounding', '--disable-backgrounding-occluded-windows']
    fs.mkdirSync(profile, { recursive: true })
    const child = spawn(found.executable, args, { stdio: ['ignore', 'ignore', 'ignore', 'pipe', 'pipe'], windowsHide: false })
    await new Promise<void>((resolve, reject) => {
      child.once('error', reject)
      child.once('spawn', resolve)
    }).catch((error) => { bootstrap.close(); throw error })
    this.pipe = new MusicPipe(child.stdio[3] as Writable, child.stdio[4] as Readable)
    const pipe = this.pipe
    if (this.stopped) { bootstrap.close(); this.stop(); throw new Error('Music widget closed.') }
    child.once('exit', () => {
      pipe.close()
      if (this.pipe !== pipe) return
      this.playbackSession = ''
      if (this.timer) clearInterval(this.timer)
      this.timer = null
      this.host?.destroy()
      this.host = null
      if (!this.stopped) this.changed(null, 'Background player closed. Press Play to reconnect.')
    })
    if (this.identity.browser !== found.browser) this.identity.appId = ''
    this.identity.browser = found.browser
    this.pid = child.pid!
    this.save()
    try {
      let target: any
      for (let i = 0; i < 30; i++) {
        const { targetInfos } = await pipe.send('Target.getTargets')
        target = targetInfos.find((item: any) => item.type === 'page' && item.url === bootstrapUrl)
        if (target) break
        await new Promise((resolve) => setTimeout(resolve, 100))
      }
      if (!target) throw new Error('Could not create the background music page')
      this.playbackId = target.targetId
      this.playbackSession = await this.attach(this.playbackId)
      await pipe.send('Emulation.setFocusEmulationEnabled', { enabled: true }, this.playbackSession)
      await this.evaluate(this.playbackSession, `document.title=${JSON.stringify(title)}; true`)
      let hidden = false
      for (let i = 0; i < 30; i++) {
        if (hideMusicBackgroundWindow(this.pid, title)) { hidden = true; break }
        await new Promise((resolve) => setTimeout(resolve, 100))
      }
      if (!hidden) throw new Error('Could not hide the background player window')
      const dimensions = await this.evaluate(this.playbackSession, '({top:Math.max(0,outerHeight-innerHeight)*devicePixelRatio,side:Math.max(0,outerWidth-innerWidth)*devicePixelRatio/2})')
      if (this.stopped) throw new Error('Music widget closed.')
      this.host = this.createHost?.(this.pid, { top: Math.round(dimensions.top), side: Math.round(dimensions.side) }, () => this.stop()) || null
      bootstrap.close()
      await pipe.send('Page.navigate', { url: this.resume.videoId ? `https://music.youtube.com/watch?v=${this.resume.videoId}` : 'https://music.youtube.com/' }, this.playbackSession)
      this.timer = setInterval(() => { void this.poll() }, 1000)
      void this.poll()
    } catch (error) {
      bootstrap.close()
      pipe.close()
      this.playbackSession = ''
      throw new Error(`Could not start background music. Close the previous dedicated player window and try again. ${error instanceof Error ? error.message : ''}`)
    }
  }

  private async attach(targetId: string): Promise<string> {
    const existing = this.sessions.get(targetId)
    if (existing) return existing
    const { sessionId } = await this.pipe!.send('Target.attachToTarget', { targetId, flatten: true })
    this.sessions.set(targetId, sessionId)
    return sessionId
  }

  private async evaluate(session: string, expression: string, userGesture = false): Promise<any> {
    const result = await this.pipe!.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true, userGesture }, session)
    if (result.exceptionDetails) {
      const message = result.exceptionDetails.exception?.description || result.exceptionDetails.text || 'Playback control failed'
      throw new Error(message.split('\n')[0].replace(/^Error:\s*/, ''))
    }
    return result.result?.value
  }

  async open(url = 'https://music.youtube.com/'): Promise<number> {
    playerArguments(this.directory, url) // Validate navigation before contacting the browser.
    await this.ensure()
    if (url !== 'https://music.youtube.com/') await this.pipe!.send('Page.navigate', { url }, this.playbackSession)
    if (!this.host) throw new Error('Music window host is unavailable')
    this.host.show()
    return this.pid
  }

  hide(): void { this.host?.hide() }

  async instanceInfo(): Promise<{ instanceId: string; pageCount: number }> {
    if (!this.ready) return { instanceId: '', pageCount: 0 }
    const { targetInfos } = await this.pipe!.send('Target.getTargets')
    return { instanceId: this.playbackId, pageCount: targetInfos.filter((target: any) => target.type === 'page').length }
  }

  async play(videoId: string, position = 0): Promise<void> {
    if (!/^[\w-]{11}$/.test(videoId)) throw new Error('Invalid track')
    const request = ++this.playbackRequest
    await this.ensure()
    if (request !== this.playbackRequest) return
    this.changing = true
    this.resume = { videoId, position }
    this.save()
    try {
      const navigation = new URL(`https://music.youtube.com/watch?v=${videoId}`)
      if (this.current?.playlistId) navigation.searchParams.set('list', this.current.playlistId)
      let queueSelection = false
      if (this.current?.videoId !== videoId) {
        let selected = false
        if (this.current?.queue.some((item) => item.videoId === videoId)) {
          selected = await this.evaluate(this.playbackSession, musicPageSelect(videoId), true)
        }
        queueSelection = selected
        if (!selected) {
          await this.pipe!.send('Page.navigate', { url: navigation.href }, this.playbackSession)
        }
      }
      let state: PlayerPageState | null = null
      const until = Date.now() + 25_000
      const selectionTime = Date.now()
      while (Date.now() < until) {
        if (request !== this.playbackRequest) return
        try { state = await this.evaluate(this.playbackSession, MUSIC_PAGE_STATE) } catch { /* document navigation */ }
        if (state?.available && state.videoId === videoId) break
        if (queueSelection && state?.videoId !== videoId && Date.now() - selectionTime > 1500) {
          queueSelection = false
          await this.pipe!.send('Page.navigate', { url: navigation.href }, this.playbackSession)
        }
        await new Promise((resolve) => setTimeout(resolve, 250))
      }
      if (!state?.available || state.videoId !== videoId) throw new Error('This track did not load. Browse music to check sign-in or playback availability.')
      if (request !== this.playbackRequest) return
      if (position > 0) await this.evaluate(this.playbackSession, musicPageCommand('seek', position), true)
      for (let attempt = 0; ; attempt++) {
        try { await this.evaluate(this.playbackSession, musicPageCommand('play'), true); break }
        catch (error) {
          if (attempt >= 4 || !/AbortError|new load|interrupted/i.test(String(error))) throw error
          await new Promise((resolve) => setTimeout(resolve, 300))
        }
      }
      this.resume = { videoId, position }
      this.save()
      await this.publish()
    } finally { if (request === this.playbackRequest) this.changing = false }
  }

  async command(action: string, value?: number): Promise<void> {
    await this.ensure()
    if (!this.current?.available && this.resume.videoId && action !== 'pause') {
      await this.play(this.resume.videoId, this.resume.position)
      return
    }
    await this.evaluate(this.playbackSession, musicPageCommand(action, value), true)
    await this.publish()
  }

  private async publish(): Promise<void> {
    const state = await this.evaluate(this.playbackSession, MUSIC_PAGE_STATE) as PlayerPageState | null
    if (!state || this.stopped) return
    // A queue transition briefly clears the page before its next track loads.
    // Keep the existing cover strip until a usable replacement is available.
    if (this.current?.title && (!state.title || (state.videoId !== this.current.videoId && !state.available))) return
    this.current = state
    if (/^[\w-]{11}$/.test(state.videoId) && state.available) {
      const changed = this.resume.videoId !== state.videoId
      this.resume = { videoId: state.videoId, position: state.position }
      if (changed) this.save()
    }
    this.changed(state)
  }

  private async poll(): Promise<void> {
    if (this.polling || this.changing || !this.ready) return
    this.polling = true
    try {
      await this.publish()
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Player connection interrupted'
      if (!this.stopped && this.ready && !/Execution context|Cannot find context|Inspected target navigated/i.test(message)) this.changed(this.current, message)
    } finally { this.polling = false }
  }

  stop(): void {
    this.stopped = true
    ++this.playbackRequest
    if (this.timer) clearInterval(this.timer)
    this.timer = null
    this.save()
    const pipe = this.pipe
    this.pipe = null
    this.playbackSession = ''
    this.playbackId = ''
    this.sessions.clear()
    this.current = null
    this.changing = false
    this.host?.destroy()
    this.host = null
    if (pipe && !pipe.closed) {
      const closing = pipe.send('Browser.close').catch(() => {}).finally(() => {
        pipe.close()
        if (this.closing === closing) this.closing = null
      })
      this.closing = closing
    }
    this.changed(null)
  }
}
