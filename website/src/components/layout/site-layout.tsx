import { useEffect } from 'react'
import { Outlet, useLocation } from 'react-router'

import { Footer } from '@/components/layout/footer'
import { Nav } from '@/components/layout/nav'
import { usePageMeta } from '@/hooks/use-page-meta'

/**
 * On navigation: jump to the anchor if the URL carries one, otherwise back to
 * the top, and move keyboard focus into the main landmark.
 *
 * That last part matters — without it focus stays on the link that was
 * clicked, so a screen reader never announces that the page changed.
 *
 * ponytail: replaces react-router's <ScrollRestoration>, which only works
 * under a data router. Restoring exact scroll offsets on back/forward would
 * mean adopting createBrowserRouter for four static pages.
 */
const useNavigationEffects = () => {
  const { pathname, hash } = useLocation()

  useEffect(() => {
    if (hash !== '') {
      document.querySelector(hash)?.scrollIntoView()
    } else {
      window.scrollTo(0, 0)
    }
    document.getElementById('main')?.focus({ preventScroll: true })
  }, [pathname, hash])
}

export const SiteLayout = () => {
  useNavigationEffects()
  usePageMeta()

  return (
    <>
      <a
        href="#main"
        className="sr-only rounded-md border border-line bg-panel px-4 py-2 focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-100"
      >
        Skip to content
      </a>
      <Nav />
      <main id="main" tabIndex={-1} className="outline-none">
        <Outlet />
      </main>
      <Footer />
    </>
  )
}
