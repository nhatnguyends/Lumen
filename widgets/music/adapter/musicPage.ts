import type { BrowserMusicState } from './musicBrowser'

export interface PlayerPageState extends BrowserMusicState {
  videoId: string
  canSeek: boolean
  signedIn: boolean | null
  authKnown?: boolean
  playlistId: string
}

/** Runs only inside the dedicated music.youtube.com page, never Google sign-in. */
export const MUSIC_PAGE_STATE = `(() => {
  if (location.origin !== 'https://music.youtube.com') return null;
  const text = n => n?.textContent?.trim() || '';
  const bar = document.querySelector('ytmusic-player-bar');
  const video = document.querySelector('video');
  const meta = navigator.mediaSession?.metadata;
  const ad = !!document.querySelector('.ad-showing');
  const api = document.getElementById?.('movie_player');
  const activeId = api?.getVideoData?.()?.video_id || bar?.data?.videoId || '';
  const panel = document.querySelector('ytmusic-player-queue ytmusic-playlist-panel-renderer, ytmusic-player-queue');
  const rows = [...(panel || document).querySelectorAll('ytmusic-player-queue-item')]
    .filter(row => !row.closest?.('#counterpart-renderer, [slot="counterpart"]'));
  const contents = panel?.data?.contents || panel?.data?.playlistPanelRenderer?.contents;
  const unwrap = item => {
    const w = item?.playlistPanelVideoWrapperRenderer;
    if (!w) return item?.playlistPanelVideoRenderer || item;
    const primary = w.primaryRenderer?.playlistPanelVideoRenderer;
    const alts = (w.counterpart || []).map(c => c?.counterpartRenderer?.playlistPanelVideoRenderer).filter(Boolean);
    return alts.find(c => (activeId && c.videoId === activeId) || (!primary?.selected && c.selected === true)) || primary;
  };
  const items = Array.isArray(contents) && contents.some(x => x?.playlistPanelVideoRenderer || x?.playlistPanelVideoWrapperRenderer)
    ? contents.map(x => ({data: unwrap(x)})) : rows;
  const runs = v => v?.runs?.map(r => r.text || '').join('') || v?.simpleText || '';
  const queue = items.slice(0, 60).flatMap(row => {
    const d = row.data?.playlistPanelVideoRenderer || row.data || {};
    const link = row.querySelector?.('a[href*="watch?v="]');
    const id = d.videoId || (link ? new URL(link.href).searchParams.get('v') : '');
    if (!/^[\\w-]{11}$/.test(id || '')) return [];
    const thumb = d.thumbnail?.thumbnails?.at(-1)?.url || row.querySelector?.('img')?.src || '';
    return [{videoId:id,title:runs(d.title) || text(row.querySelector?.('.song-title, #video-title')) || text(link),
      artist:runs(d.shortBylineText) || text(row.querySelector?.('.byline')),thumb,duration:0,
      url:'https://music.youtube.com/watch?v='+id, selected:row.hasAttribute?.('selected') || d.selected === true}];
  });
  const videoId = activeId || queue.find(t => t.selected)?.videoId || new URL(location.href).searchParams.get('v') || '';
  const current = queue.find(t => t.videoId === videoId);
  for (const item of queue) item.selected = item.videoId === videoId;
  const title = current?.title || text(bar?.querySelector('.title')) || meta?.title;
  const loggedIn = window.ytcfg?.get?.('LOGGED_IN') ?? window.ytcfg?.data_?.LOGGED_IN;
  const avatar = !!document.querySelector('ytmusic-user-menu-renderer #avatar, ytmusic-user-menu-renderer #avatar-btn, ytmusic-nav-bar #avatar, ytmusic-nav-bar #avatar-btn, ytmusic-nav-bar #avatar-button');
  const signIn = !!document.querySelector('ytmusic-nav-bar a[href*="accounts.google.com"], ytmusic-nav-bar a[href*="ServiceLogin"]');
  const signedIn = typeof loggedIn === 'boolean' ? loggedIn : avatar ? true : signIn ? false : null;
  const duration = Number.isFinite(video?.duration) ? video.duration : 0;
  return { available:!!video && video.readyState >= 2 && !!title, playing:!!video && !video.paused && !video.ended,
    title, artist:current?.artist || text(bar?.querySelector('.byline a')) || meta?.artist,
    album:meta?.album || '', art:current?.thumb || bar?.querySelector('img')?.src || meta?.artwork?.at(-1)?.src || null,
    position:video?.currentTime || 0,duration,canSeek:!ad && duration > 0,
    canNext:!ad && !!bar?.querySelector('.next-button:not([disabled])'),
    canPrevious:!ad && !!bar?.querySelector('.previous-button:not([disabled])'),
    videoId,
    playlistId:new URL(location.href).searchParams.get('list') || '',
    signedIn,
    queue,error:null};
})()`

export function musicPageSelect(videoId: string): string {
  if (!/^[\w-]{11}$/.test(videoId)) throw new Error('Invalid track')
  return `(() => {
    if (location.origin !== 'https://music.youtube.com') return false;
    const id=${JSON.stringify(videoId)};
    const row=[...document.querySelectorAll('ytmusic-player-queue-item')].find(row => {
      const data=row.data?.playlistPanelVideoRenderer || row.data || {};
      const link=row.querySelector('a[href*="watch?v="]');
      return (data.videoId || (link ? new URL(link.href).searchParams.get('v') : '')) === id;
    });
    if (!row) return false;
    const button=row.querySelector('#play-button, .play-button, ytmusic-play-button-renderer button');
    if (button) button.click();
    else row.dispatchEvent(new MouseEvent('dblclick',{bubbles:true,cancelable:true,composed:true}));
    return true;
  })()`
}

export function musicPageCommand(action: string, value?: number): string {
  if (!['toggle', 'play', 'pause', 'next', 'previous', 'seek'].includes(action)) throw new Error('Unknown playback control')
  return `(async () => {
    if (location.origin !== 'https://music.youtube.com') throw new Error('Music player is still loading');
    const action=${JSON.stringify(action)}, value=${JSON.stringify(Number.isFinite(value) ? value : null)};
    const video=document.querySelector('video');
    if (!video) throw new Error('Choose a song first');
    if (action === 'play' || (action === 'toggle' && video.paused)) await video.play();
    else if (action === 'pause' || action === 'toggle') video.pause();
    else if (action === 'seek' && value !== null && Number.isFinite(video.duration) && !document.querySelector('.ad-showing')) video.currentTime=Math.max(0,Math.min(video.duration,value));
    else if (action === 'next' || action === 'previous') {
      const button=document.querySelector('ytmusic-player-bar .' + action + '-button:not([disabled])');
      if (!button || document.querySelector('.ad-showing')) throw new Error('This control is unavailable for the current track');
      button.click();
    }
    return true;
  })()`
}
