// 外部連結的圖示：依主機名（含子網域）對應 UiIcon 名稱，其餘一律 link
const HOSTS: [string, string[]][] = [
  ['instagram', ['instagram.com']], ['threads', ['threads.net', 'threads.com']], ['youtube', ['youtube.com', 'youtu.be']],
  ['line', ['line.me', 'lin.ee']], ['facebook', ['facebook.com', 'fb.com', 'fb.me']], ['tiktok', ['tiktok.com']],
  ['x', ['x.com', 'twitter.com']], ['github', ['github.com']],
]
export function linkIcon(url: string): string {
  let host: string
  try { host = new URL(url).hostname.toLowerCase() } catch { return 'link' }
  return HOSTS.find(([, ds]) => ds.some(d => host === d || host.endsWith(`.${d}`)))?.[0] ?? 'link'
}
