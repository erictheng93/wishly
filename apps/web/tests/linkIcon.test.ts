import { expect, it } from 'vitest'
import { linkIcon } from '../app/utils/linkIcon'

it.each([
  ['https://www.instagram.com/x', 'instagram'], ['https://youtu.be/x', 'youtube'], ['https://line.me/R/ti/p/x', 'line'],
  ['https://x.com/a', 'x'], ['https://github.com/a', 'github'], ['https://notinstagram.com/a', 'link'],
  ['https://example.com', 'link'], ['not a url', 'link'],
])('%s -> %s', (url, icon) => { expect(linkIcon(url)).toBe(icon) })
