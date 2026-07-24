import { useEffect } from 'react'
import { useLocation } from 'react-router'

import { metaFor } from '@/lib/routes'

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
