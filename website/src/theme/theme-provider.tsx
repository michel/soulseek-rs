import { useCallback, useEffect, useMemo, useSyncExternalStore, type ReactNode } from 'react'

import { ThemeContext, type Theme, type ThemeContextValue } from './theme-context'
import {
  readResolvedTheme,
  readTheme,
  serverResolvedTheme,
  serverTheme,
  subscribe,
  writeTheme,
} from './theme-store'

interface ThemeProviderProps {
  children: ReactNode
}

export const ThemeProvider = ({ children }: ThemeProviderProps) => {
  const theme = useSyncExternalStore(subscribe, readTheme, serverTheme)
  const resolvedTheme = useSyncExternalStore(
    subscribe,
    readResolvedTheme,
    serverResolvedTheme,
  )

  useEffect(() => {
    const root = document.documentElement
    root.dataset['theme'] = resolvedTheme
    // Paints native widgets (scrollbars, form controls) to match, which the
    // attribute alone does not do.
    root.style.colorScheme = resolvedTheme
  }, [resolvedTheme])

  const setTheme = useCallback((next: Theme) => {
    writeTheme(next)
  }, [])

  const value = useMemo<ThemeContextValue>(
    () => ({ theme, resolvedTheme, setTheme }),
    [theme, resolvedTheme, setTheme],
  )

  return <ThemeContext value={value}>{children}</ThemeContext>
}
