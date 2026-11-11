import type { Readable, Writable } from 'node:stream'

/** Private inherited pipes to Lumen's own browser process; no listening port. */
export class MusicPipe {
  private sequence = 0
  private buffer = ''
  private pending = new Map<number, { resolve(value: any): void; reject(error: Error): void; timer: NodeJS.Timeout }>()
  closed = false
  constructor(private readonly input: Writable, output: Readable) {
    output.setEncoding('utf8')
    output.on('data', (chunk: string) => {
      this.buffer += chunk
      let at: number
      while ((at = this.buffer.indexOf('\0')) >= 0) {
        const line = this.buffer.slice(0, at)
        this.buffer = this.buffer.slice(at + 1)
        if (!line) continue
        try {
          const message = JSON.parse(line)
          const pending = this.pending.get(message.id)
          if (!pending) continue
          clearTimeout(pending.timer)
          this.pending.delete(message.id)
          if (message.error) pending.reject(new Error(message.error.message))
          else pending.resolve(message.result)
        } catch { /* ignore an unrelated event */ }
      }
    })
    output.on('end', () => this.close())
    output.on('error', () => this.close())
    input.on('error', () => this.close())
  }
  send(method: string, params: Record<string, unknown> = {}, sessionId?: string): Promise<any> {
    if (this.closed) return Promise.reject(new Error('Music browser disconnected'))
    const id = ++this.sequence
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`Music player timed out (${method})`))
      }, 12_000)
      this.pending.set(id, { resolve, reject, timer })
      this.input.write(`${JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) })}\0`, (error) => {
        if (error) this.close()
      })
    })
  }
  close(): void {
    if (this.closed) return
    this.closed = true
    for (const item of this.pending.values()) { clearTimeout(item.timer); item.reject(new Error('Music browser disconnected')) }
    this.pending.clear()
    this.input.end()
  }
}
