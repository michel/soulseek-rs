import { StrictMode } from 'react'
import { renderToString } from 'react-dom/server'
import { StaticRouter } from 'react-router'

import { App } from '@/App'
import { ThemeProvider } from '@/theme/theme-provider'

export const render = (location: string): string =>
  renderToString(
    <StrictMode>
      <ThemeProvider>
        <StaticRouter location={location}>
          <App />
        </StaticRouter>
      </ThemeProvider>
    </StrictMode>,
  )
