let api: any = null
const windows = new Map<number, unknown>()
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
      position: user32.func('bool __stdcall SetWindowPos(void *window, void *after, int x, int y, int width, int height, uint32 flags)')
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
  // Only explicit Browse restores/focuses the app; playback remains unobtrusive.
  api.show(window, 7)
  return {
    show() { if (!disposed) { api.show(window, 9); reachable(); api.foreground(window) } },
    hide() { if (!disposed) api.show(window, 6) },
    destroy() {
      if (disposed) return
      disposed = true
      api.show(window, 0)
      windows.delete(pid)
    }
  }
}

