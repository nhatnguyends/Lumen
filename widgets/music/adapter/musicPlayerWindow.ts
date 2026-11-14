import { deflateSync } from 'node:zlib'

let api: any = null
let icons: { big: unknown; small: unknown } | null = null
const windows = new Map<number, unknown>()

function crc32(buf: Buffer): number {
  let c = 0xffffffff
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i]
    for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (c & 1 ? 0xedb88320 : 0)
  }
  return (c ^ 0xffffffff) >>> 0
}

function pngChunk(type: string, data: Buffer): Buffer {
  const len = Buffer.alloc(4)
  len.writeUInt32BE(data.length, 0)
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(body), 0)
  return Buffer.concat([len, body, crc])
}

function triangleSdf(px: number, py: number, ax: number, ay: number, bx: number, by: number, cx: number, cy: number): number {
  const edge = (x1: number, y1: number, x2: number, y2: number) => {
    const ex = x2 - x1, ey = y2 - y1
    const wx = px - x1, wy = py - y1
    const t = Math.max(0, Math.min(1, (wx * ex + wy * ey) / (ex * ex + ey * ey)))
    const dx = wx - ex * t, dy = wy - ey * t
    return { d2: dx * dx + dy * dy, s: ex * wy - ey * wx }
  }
  const e1 = edge(ax, ay, bx, by)
  const e2 = edge(bx, by, cx, cy)
  const e3 = edge(cx, cy, ax, ay)
  const minD = Math.sqrt(Math.min(e1.d2, e2.d2, e3.d2))
  const inside = (e1.s >= 0 && e2.s >= 0 && e3.s >= 0) || (e1.s <= 0 && e2.s <= 0 && e3.s <= 0)
  return inside ? -minD : minD
}

function renderYouTubeMusicPng(size: number): Buffer {
  const stride = size * 4 + 1
  const raw = Buffer.alloc(stride * size)
  const c = size * 0.5
  const outerR = size * 0.45
  const ringR = size * 0.275
  const ringHalfW = Math.max(1, size * 0.024)
  const triL = c - size * 0.075
  const triR = c + size * 0.135
  const triH = size * 0.125

  for (let y = 0; y < size; y++) {
    const row = y * stride
    raw[row] = 0
    for (let x = 0; x < size; x++) {
      const dx = x + 0.5 - c
      const dy = y + 0.5 - c
      const dist = Math.hypot(dx, dy)
      const outerA = Math.max(0, Math.min(1, outerR - dist + 0.6))
      if (outerA <= 0) continue

      const ringDist = Math.abs(dist - ringR) - ringHalfW
      const ringA = Math.max(0, Math.min(1, 0.6 - ringDist))
      const triDist = triangleSdf(x + 0.5, y + 0.5, triL, c - triH, triR, c, triL, c + triH)
      const triA = Math.max(0, Math.min(1, 0.6 - triDist))
      const whiteA = Math.max(ringA, triA)

      const r = Math.round(255)
      const g = Math.round(whiteA * 255)
      const b = Math.round(whiteA * 255 + (1 - whiteA) * 32)
      const off = row + 1 + x * 4
      raw[off] = r
      raw[off + 1] = g
      raw[off + 2] = b
      raw[off + 3] = Math.round(outerA * 255)
    }
  }

  const ihdr = Buffer.alloc(13)
  ihdr.writeUInt32BE(size, 0)
  ihdr.writeUInt32BE(size, 4)
  ihdr[8] = 8
  ihdr[9] = 6 // RGBA
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    pngChunk('IHDR', ihdr),
    pngChunk('IDAT', deflateSync(raw)),
    pngChunk('IEND', Buffer.alloc(0)),
  ])
}

function ensureIcons(): { big: unknown; small: unknown } | null {
  if (icons || !api) return icons
  const make = (size: number) => {
    const png = renderYouTubeMusicPng(size)
    return api.createIcon(png, png.length, true, 0x00030000, size, size, 0)
  }
  const big = make(128)
  const small = make(32)
  if (!big && !small) return null
  icons = { big, small }
  return icons
}

function applyWindowIcon(window: unknown): void {
  const loaded = ensureIcons()
  if (!loaded || !api) return
  if (loaded.big) api.sendMessage(window, 0x0080, 1, loaded.big)
  if (loaded.small) api.sendMessage(window, 0x0080, 0, loaded.small)
}
/** Hide only the uniquely titled bootstrap window of the browser we launched. */
export function hideMusicBackgroundWindow(pid: number, title: string): boolean {
  if (process.platform !== 'win32' || !Number.isInteger(pid) || pid <= 0 || !/^Lumen Music Background [a-f0-9]{16}$/.test(title)) return false
  if (!api) {
    const koffi = require('koffi')
    const user32 = koffi.load('user32.dll')
    const callback = koffi.proto('bool __stdcall LumenMusicWindowCallback(void *window, intptr_t parameter)')
    api = {
      enumerate: user32.func('EnumWindows', 'bool', [koffi.pointer(callback), 'intptr_t']),
      processId: user32.func('uint32 __stdcall GetWindowThreadProcessId(void *window, uint32 *processId)'),
      windowText: user32.func('int __stdcall GetWindowTextW(void *window, void *text, int count)'),
      show: user32.func('bool __stdcall ShowWindow(void *window, int command)'),
      windowRect: user32.func('bool __stdcall GetWindowRect(void *window, void *rect)'),
      monitor: user32.func('void * __stdcall MonitorFromWindow(void *window, uint32 flags)'),
      monitorInfo: user32.func('bool __stdcall GetMonitorInfoW(void *monitor, void *info)'),
      foreground: user32.func('bool __stdcall SetForegroundWindow(void *window)'),
      position: user32.func('bool __stdcall SetWindowPos(void *window, void *after, int x, int y, int width, int height, uint32 flags)'),
      createIcon: user32.func('void * __stdcall CreateIconFromResourceEx(uint8 *presbits, uint32 dwResSize, bool fIcon, uint32 dwVer, int cxDesired, int cyDesired, uint32 Flags)'),
      sendMessage: user32.func('intptr_t __stdcall SendMessageW(void *window, uint32 msg, uintptr_t wParam, void *lParam)')
    }
  }
  const { enumerate, processId, windowText, show } = api
  let hidden = false
  enumerate((window: unknown) => {
    const owner = new Uint32Array(1)
    processId(window, owner)
    if (owner[0] !== pid) return true
    const text = Buffer.alloc(2048)
    const length = windowText(window, text, 1024)
    const name = text.toString('utf16le', 0, length * 2)
    if (name !== title && !name.startsWith(`${title} - `)) return true
    show(window, 0)
    windows.set(pid, window)
    hidden = true
    return false
  }, 0)
  return hidden
}

export interface MusicFrameInsets { top: number; side: number }

/** Keep keyboard/IME, chrome and live resize in the browser's own window/thread,
 * with no cross-process parent frame or cropped child-window bounds. */
export function createMusicWindowHost(pid: number) {
  const window = windows.get(pid)
  if (!window || !api) throw new Error('Music window is not ready')
  let disposed = false
  const reachable = () => {
    const bounds = Buffer.alloc(16), info = Buffer.alloc(40)
    info.writeUInt32LE(40)
    if (!api.windowRect(window, bounds) || !api.monitorInfo(api.monitor(window, 2), info)) throw new Error('Could not place the music window')
    const [left, top, right, bottom] = [20, 24, 28, 32].map(offset => info.readInt32LE(offset))
    const [x, y, r] = [0, 4, 8].map(offset => bounds.readInt32LE(offset))
    if (Math.min(r, right) - Math.max(x, left) >= 80 && y >= top - 16 && y + 32 <= bottom) return
    const width = Math.min(1100, right - left), height = Math.min(800, bottom - top)
    api.position(window, null, left + Math.round((right - left - width) / 2), top + Math.round((bottom - top - height) / 2), width, height, 0x0014)
  }
  reachable()
  applyWindowIcon(window)
  // Only explicit Browse restores/focuses the app; playback remains unobtrusive.
  api.show(window, 7)
  return {
    show() { if (!disposed) { applyWindowIcon(window); api.show(window, 9); reachable(); api.foreground(window) } },
    hide() { if (!disposed) api.show(window, 6) },
    refreshIcon() { if (!disposed) applyWindowIcon(window) },
    destroy() {
      if (disposed) return
      disposed = true
      api.show(window, 0)
      windows.delete(pid)
    }
  }
}

