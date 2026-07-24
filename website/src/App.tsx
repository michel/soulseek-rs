import type { ReactElement } from 'react'
import { Route, Routes } from 'react-router'

import { SiteLayout } from '@/components/layout/site-layout'
import { ROUTES, type RoutePath } from '@/lib/routes'
import { Community } from '@/pages/community'
import { Docs } from '@/pages/docs'
import { Home } from '@/pages/home'
import { Install } from '@/pages/install'
import { NotFound } from '@/pages/not-found'

const PAGES: Record<RoutePath, ReactElement> = {
  '/': <Home />,
  '/install': <Install />,
  '/docs': <Docs />,
  '/community': <Community />,
}

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
