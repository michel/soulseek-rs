/*
 * eslint-plugin-jsx-a11y ships no types, and TS's inference from its JS picks
 * up the legacy eslintrc `configs` shape (plugins as string[]) instead of the
 * flat-config one. Declaring the flat export is enough to type the one thing
 * eslint.config.ts touches.
 */
declare module 'eslint-plugin-jsx-a11y' {
  import type { Linter } from 'eslint'

  export const flatConfigs: {
    recommended: Linter.Config
    strict: Linter.Config
  }
}
