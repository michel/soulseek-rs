export interface RouteMeta {
  /** Path without the base prefix, always leading-slashed. */
  path: string
  title: string
  description: string
}

/**
 * The single source of truth for what pages exist.
 *
 * Read three times: React Router matches against it, the layout applies the
 * meta on navigation, and scripts/prerender.ts walks it to decide which HTML
 * files to write. Adding a page means adding one entry here plus its route.
 */
/*
 * `as const satisfies` rather than a plain annotation: the annotation widened
 * `path` to `string`, which is what let App.tsx's hand-written route list
 * drift from this one without failing the build. Literal paths make the
 * mapping in App.tsx exhaustive-checkable.
 */
export const ROUTES = [
  {
    path: '/',
    title: 'soulseek-rs, a Soulseek client for your terminal',
    description:
      'soulseek-rs is a keyboard-driven Soulseek client and a Rust protocol library. Search, share, browse, and chat from your terminal.',
  },
  {
    path: '/install',
    title: 'soulseek-rs, install',
    description:
      'Install the soulseek-rs client with cargo, build it from source, or depend on the protocol library and write your own client.',
  },
  {
    path: '/docs',
    title: 'soulseek-rs, docs',
    description:
      'Quick start for soulseek-rs: install, run, drive it from the keyboard, and use the shell subcommands.',
  },
  {
    path: '/community',
    title: 'soulseek-rs, community',
    description:
      'How to help with soulseek-rs, and where to go when another Soulseek client fits you better. MIT licensed. No telemetry.',
  },
] as const satisfies readonly RouteMeta[]

/** Every path the site serves, as literals. */
export type RoutePath = (typeof ROUTES)[number]['path']

export const NOT_FOUND_META: RouteMeta = {
  path: '/404',
  title: 'soulseek-rs, not found',
  description: 'That page does not exist.',
}

export const metaFor = (pathname: string): RouteMeta => {
  const trimmed =
    pathname.length > 1 && pathname.endsWith('/') ? pathname.slice(0, -1) : pathname
  return ROUTES.find((route) => route.path === trimmed) ?? NOT_FOUND_META
}
