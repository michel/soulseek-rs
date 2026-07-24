# website

The soulseek-rs project site. Vite + React + TypeScript + Tailwind v4,
deployed to GitHub Pages by `.github/workflows/pages.yml`.

```bash
bun install
bun run dev        # http://localhost:5173/soulseek-rs/
bun run build      # client build, SSR build, then prerender
bun run typecheck
bun run lint
```

## How the static site is produced

`bun run build` runs three steps:

1. `vite build` — the client bundle, plus `dist/index.html` as the template.
2. `vite build --ssr src/entry-server.tsx` — the same app compiled for Node.
3. `bun scripts/prerender.ts` — renders every route in `src/lib/routes.ts` to
   its own file (`dist/install/index.html`, and so on) with that route's title
   and description substituted in, plus a `404.html`.

Without step 3 GitHub Pages has no file at `/install` and answers a direct link
with a 404. With it, every URL returns 200 with fully rendered markup, and the
client hydrates that markup rather than replacing it.

Adding a page means: a component in `src/pages/`, a `<Route>` in `src/App.tsx`,
and an entry in `src/lib/routes.ts`. The last one is what the prerenderer walks.

## Design system

Tokens live in `src/index.css`, ported from the Claude Design project's
`tokens/colors.css`. They are the same values that back the TUI in
`soulseek-rs/src/ui/styles.rs` — change one, change both.

Dark is the native theme; light re-cuts the semantic hues roughly 10% darker,
because phosphor and tape are legible on vinyl but wash out on manila. Theme
selection is `[data-theme]` on `<html>`, defaulting to the OS setting, with an
inline script in `index.html` applying it before first paint so the page never
flashes the wrong background.

## Notes

- `base` in `vite.config.ts` is `/soulseek-rs/` because this is a project site.
  Pointing a custom domain at it means setting that to `/` (the router reads
  `import.meta.env.BASE_URL`, so it follows automatically).
- Copy in `src/pages/` follows the project's tone of voice: plainspoken,
  precise, honest about limits first. No copyrighted artist or album names
  anywhere, including the TUI demo data in `src/components/tui/data.ts`.
