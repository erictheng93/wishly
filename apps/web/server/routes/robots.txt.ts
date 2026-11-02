import { robotsTxt } from '../../app/utils/seo'
import { seoIndexable } from '../utils/seoIndexable'

export default defineEventHandler((event) => {
  setHeader(event, 'content-type', 'text/plain; charset=utf-8')
  setHeader(event, 'cache-control', 'public, max-age=3600')
  return robotsTxt(useRuntimeConfig(event).public.siteUrl, seoIndexable(event))
})
