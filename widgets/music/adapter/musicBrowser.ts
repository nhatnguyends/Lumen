import { createServer, type Server } from 'node:http'
import { randomBytes, timingSafeEqual } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import type { MusicQueueItem } from './types'

export const MUSIC_BROWSER_PORT = 19347
const STALE_MS = 8_000

export interface BrowserMusicState {
  available: boolean
  playing: boolean
  title: string
  artist: string
  album: string
  art: string | null
  position: number
  duration: number
  canNext: boolean
  canPrevious: boolean
  queue: MusicQueueItem[]
  error: string | null
}

export interface BrowserCommand {
  action: string
  value?: number | string
}

function cleanText(value: unknown): string {
  return typeof value === 'string' ? value.slice(0, 1000) : ''
}
function seconds(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? Math.max(0, value) : 0
}
function artwork(value: unknown): string | null {
  try {
    const url = new URL(cleanText(value))
    return url.protocol === 'https:' ? url.href : null
  } catch { return null }
}

/** Only the explicitly paired extension can publish a tab or receive controls. */
export class MusicBrowserBridge {
  private server: Server | null = null
  private timer: NodeJS.Timeout | null = null
  private lastSeen = 0
  private pending: { command: BrowserCommand; at: number }[] = []
  private state: BrowserMusicState | null = null
  private pairing: { ticket: string; expires: number } | null = null
  error: string | null = null

  constructor(
    readonly token: string,
    private readonly changed: (state: BrowserMusicState | null) => void,
    private readonly extensionPath = ''
  ) {}

  /** A short-lived, single-use invitation; the permanent token stays out of URLs. */
  beginPairing(): string {
    const ticket = randomBytes(24).toString('hex')
    this.pairing = { ticket, expires: Date.now() + 5 * 60_000 }
    const port = (this.server?.address() as { port: number } | null)?.port
    if (!port) throw new Error('Browser helper is not ready. Try again in a moment.')
    return `http://127.0.0.1:${port}/connect#${ticket}`
  }

  get connected(): boolean { return this.state !== null && Date.now() - this.lastSeen < STALE_MS }
  get current(): BrowserMusicState | null { return this.connected ? this.state : null }

  send(command: BrowserCommand): boolean {
    if (!this.connected || this.pending.length >= 20) return false
    this.pending.push({ command, at: Date.now() })
    return true
  }

  start(port = MUSIC_BROWSER_PORT): Promise<void> {
    if (this.server) return Promise.resolve()
    return new Promise((resolve, reject) => {
      const server = createServer(async (req, res) => {
        // Reject DNS rebinding and ordinary web origins, even with a leaked token.
        const origin = req.headers.origin
        if (req.headers.host !== `127.0.0.1:${(server.address() as { port: number })?.port}` ||
            (origin && !/^chrome-extension:\/\/[a-p]{32}$/.test(origin))) {
          res.writeHead(403).end(); return
        }
        if (req.method === 'GET' && (req.url === '/connect' || req.url === '/connect.js')) {
          try {
            const isPage = req.url === '/connect'
            const file = path.join(this.extensionPath, isPage ? 'connect.html' : 'connect.js')
            const escape = (value: string) => value.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!)
            const body = fs.readFileSync(file, 'utf8').replace('__EXTENSION_PATH__', escape(this.extensionPath))
            res.writeHead(200, {
              'Content-Type': isPage ? 'text/html; charset=utf-8' : 'text/javascript; charset=utf-8',
              'Cache-Control': 'no-store', 'Referrer-Policy': 'no-referrer',
              'X-Content-Type-Options': 'nosniff',
              'Content-Security-Policy': "default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'"
            })
            res.end(body)
          } catch { res.writeHead(404).end() }
          return
        }
        if (req.method === 'POST' && req.url === '/v1/pair') {
          const ticket = this.pairing
          const supplied = Buffer.from(req.headers.authorization || '')
          const expected = Buffer.from(`Bearer ${ticket?.ticket || ''}`)
          if (!ticket || ticket.expires < Date.now() || supplied.length !== expected.length ||
              !timingSafeEqual(supplied, expected)) {
            res.writeHead(401).end(); return
          }
          this.pairing = null
          res.writeHead(200, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' })
          res.end(JSON.stringify({ token: this.token }))
          return
        }
        if (req.method !== 'POST' || req.url !== '/v1/sync') {
          res.writeHead(404).end(); return
        }
        const supplied = Buffer.from(req.headers.authorization || '')
        const expected = Buffer.from(`Bearer ${this.token}`)
        if (supplied.length !== expected.length || !timingSafeEqual(supplied, expected)) {
          res.writeHead(401).end(); return
        }
        try {
          let body = ''
          for await (const chunk of req) {
            body += chunk.toString('utf8')
            if (Buffer.byteLength(body) > 128 * 1024) {
              res.writeHead(413).end(); req.destroy(); return
            }
          }
          const raw = JSON.parse(body)
          if (!raw || typeof raw !== 'object' || !Array.isArray(raw.queue)) throw new Error('Invalid state')
          const queue: MusicQueueItem[] = raw.queue.slice(0, 60).flatMap((item: any) => {
            if (!item || !/^[\w-]{11}$/.test(item.videoId)) return []
            return [{
              videoId: item.videoId, title: cleanText(item.title), artist: cleanText(item.artist),
              duration: seconds(item.duration), thumb: artwork(item.thumb) || '',
              url: `https://music.youtube.com/watch?v=${item.videoId}`, selected: item.selected === true
            }]
          })
          this.state = {
            available: raw.available === true, playing: raw.playing === true,
            title: cleanText(raw.title), artist: cleanText(raw.artist), album: cleanText(raw.album),
            art: artwork(raw.art), duration: seconds(raw.duration), position: seconds(raw.position),
            canNext: raw.canNext === true, canPrevious: raw.canPrevious === true,
            queue, error: cleanText(raw.error) || null
          }
          this.lastSeen = Date.now()
          this.changed(this.state)
          const commands = this.pending.filter((item) => Date.now() - item.at < 5_000).map((item) => item.command)
          this.pending = []
          res.writeHead(200, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' })
          res.end(JSON.stringify({ commands }))
        } catch {
          res.writeHead(400).end()
        }
      })
      this.server = server
      server.requestTimeout = 5_000
      server.headersTimeout = 5_000
      server.on('error', (error) => {
        this.error = `Browser connection unavailable: ${error.message}`
        this.changed(null)
        reject(error)
      })
      server.listen(port, '127.0.0.1', () => {
        this.error = null
        this.timer = setInterval(() => {
          if (this.state && !this.connected) {
            this.state = null
            this.pending = []
            this.changed(null)
          }
        }, 1_000)
        resolve()
      })
    })
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer)
    this.timer = null
    this.server?.closeAllConnections()
    this.server?.close()
    this.server = null
    this.state = null
    this.pending = []
    this.pairing = null
  }
}

export function newMusicPairingToken(): string { return randomBytes(24).toString('hex') }
