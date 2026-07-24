import js from '@eslint/js'
import { defineConfig } from 'eslint/config'
import jsxA11y from 'eslint-plugin-jsx-a11y'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import globals from 'globals'
import tseslint from 'typescript-eslint'

export default defineConfig(
  { ignores: ['dist', 'dist-ssr', 'node_modules'] },

  js.configs.recommended,

  // strictTypeChecked over recommended: it needs type information (below) but
  // catches the whole class of bugs that only types can see — floating
  // promises, unsafe `any` flowing through, misused nullish operands.
  tseslint.configs.strictTypeChecked,
  tseslint.configs.stylisticTypeChecked,

  // `configs.flat.*`, not `configs['recommended-latest']` — the latter is
  // still the legacy eslintrc shape (plugins as a string array).
  reactHooks.configs.flat['recommended-latest'],
  jsxA11y.flatConfigs.recommended,

  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      ecmaVersion: 2023,
      globals: globals.browser,
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    plugins: {
      'react-refresh': reactRefresh,
    },
    rules: {
      'react-refresh/only-export-components': [
        'error',
        { allowConstantExport: true },
      ],

      // Unused args are usually a mistake, but an `_`-prefixed one is an
      // explicit "I know, I need the positional slot".
      '@typescript-eslint/no-unused-vars': [
        'error',
        {
          argsIgnorePattern: '^_',
          varsIgnorePattern: '^_',
          caughtErrorsIgnorePattern: '^_',
        },
      ],

      // Type-only imports must be erasable, so the bundler can drop them
      // without consulting the type graph. Pairs with verbatimModuleSyntax.
      '@typescript-eslint/consistent-type-imports': [
        'error',
        { prefer: 'type-imports', fixStyle: 'inline-type-imports' },
      ],

      // A dropped promise in an event handler fails silently in production.
      '@typescript-eslint/no-floating-promises': 'error',
      '@typescript-eslint/no-misused-promises': 'error',

      'no-console': ['error', { allow: ['warn', 'error'] }],
      eqeqeq: ['error', 'always', { null: 'ignore' }],
      'prefer-const': 'error',
      'no-var': 'error',
      'object-shorthand': 'error',
    },
  },

  // The TUI demo is a composite widget: it declares role="application" and
  // owns its keys (j/k, arrows, 1-3) the way the real terminal client does.
  // These two rules police exactly the pattern ARIA prescribes for that, so
  // they are off for this one file rather than worked around in the markup.
  // Every control inside it is still a native button or input.
  {
    files: ['src/components/tui/hero-terminal.tsx'],
    rules: {
      'jsx-a11y/no-noninteractive-tabindex': 'off',
      'jsx-a11y/no-noninteractive-element-interactions': 'off',
    },
  },

  // Config and build scripts are Node, not browser. Build output is meant to
  // be read on the terminal, so console is the interface, not a stray debug
  // statement. entry-server is a build entry point that never enters the HMR
  // graph, so the component-only export rule does not apply to it.
  {
    files: ['*.config.ts', 'scripts/**/*.ts'],
    languageOptions: {
      globals: globals.node,
    },
    rules: {
      'no-console': 'off',
      '@typescript-eslint/no-unsafe-assignment': 'off',
    },
  },
  {
    files: ['src/entry-server.tsx'],
    rules: {
      'react-refresh/only-export-components': 'off',
    },
  },
)
