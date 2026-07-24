declare module 'eslint-plugin-jsx-a11y' {
  import type { Linter } from 'eslint'

  export const flatConfigs: {
    recommended: Linter.Config
    strict: Linter.Config
  }
}
