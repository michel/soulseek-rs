import { useEffect } from 'react'
import { useLocation } from 'react-router'

import { metaFor } from '@/lib/routes'

/**
 * Keeps the title and meta description in step with the current route.
 *
 * Each route is also prerendered with its tags already in the HTML, so
 * crawlers never depend on this running; it only covers client-side
 * navigation after the first paint.
 */
export const usePageMeta = (): void => {
  const { pathname } = useLocation()

  useEffect(() => {
    const meta = metaFor(pathname)
    document.title = meta.title
    document
      .querySelector('meta[name="description"]')
      ?.setAttribute('content', meta.description)
  }, [pathname])
}
