import type { ReactElement } from 'react'
import { Route, Routes } from 'react-router'

import { SiteLayout } from '@/components/layout/site-layout'
import { ROUTES, type RoutePath } from '@/lib/routes'
import { Community } from '@/pages/community'
import { Docs } from '@/pages/docs'
import { Home } from '@/pages/home'
import { Install } from '@/pages/install'
import { NotFound } from '@/pages/not-found'

/*
 * Keyed by RoutePath, so adding an entry to ROUTES without a page here — or a
 * page here that ROUTES does not know about — fails `bun run typecheck`.
 *
 * That drift used to be silent and shipped broken both ways: a prerendered
 * file containing NotFound markup under the new page's title, or a real page
 * with no prerendered file and "not found" in its tab.
 */
const PAGES: Record<RoutePath, ReactElement> = {
  '/': <Home />,
  '/install': <Install />,
  '/docs': <Docs />,
  '/community': <Community />,
}

/*
 * Everything is eagerly imported: the whole site is a couple of hundred kB,
 * so route-splitting would trade a network round-trip on every navigation for
 * a bundle saving nobody would notice.
 */
export const App = () => (
  <Routes>
    <Route element={<SiteLayout />}>
      {ROUTES.map((route) =>
        route.path === '/' ? (
          <Route key={route.path} index element={PAGES[route.path]} />
        ) : (
          <Route key={route.path} path={route.path} element={PAGES[route.path]} />
        ),
      )}
      <Route path="*" element={<NotFound />} />
    </Route>
  </Routes>
)
