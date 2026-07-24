import { createContext } from 'react'

export const THEMES = ['light', 'dark', 'system'] as const
export type Theme = (typeof THEMES)[number]
export type ResolvedTheme = Exclude<Theme, 'system'>

export const isTheme = (value: unknown): value is Theme =>
  typeof value === 'string' && (THEMES as readonly string[]).includes(value)

export interface ThemeContextValue {
  /** What the user picked — may be 'system'. */
  theme: Theme
  /** What is actually painted right now — never 'system'. */
  resolvedTheme: ResolvedTheme
  setTheme: (theme: Theme) => void
}

/*
 * Split from the provider so this module exports only values, which keeps the
 * provider file component-only for react-refresh.
 */
export const ThemeContext = createContext<ThemeContextValue | null>(null)

/** Matches the key the design's own inline boot script reads. */
export const THEME_STORAGE_KEY = 'ssrs-theme'
