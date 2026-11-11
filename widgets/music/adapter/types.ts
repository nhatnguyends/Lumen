export interface MusicQueueItem {
  videoId: string
  title: string
  artist: string
  /** Length in seconds, or 0 when YouTube did not expose one. */
  duration: number
  thumb: string
  url: string
  /** True for the track currently playing in the queue. */
  selected?: boolean
}
