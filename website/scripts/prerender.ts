/*
 * Turns the SPA build into real static files.
 *
 * Without this, GitHub Pages has no file at /install and answers a direct
 * link with its 404 page. Writing one HTML file per route means every URL
 * returns 200 with its own title, description, and fully rendered body; the
 * client then hydrates it into the same interactive app.
 *
 * Run after both `vite build` and `vite build --ssr`.
 */
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'

import { NOT_FOUND_META, ROUTES, type RouteMeta } from '../src/lib/routes.ts'

/*
 * Held in a variable so TypeScript does not try to resolve it: the module is
 * a build artefact, and `bun run typecheck` runs before `vite build --ssr`
 * has produced it. The cast is the contract with src/entry-server.tsx.
 */
const SSR_ENTRY = '../dist-ssr/entry-server.js'
const { render } = (await import(SSR_ENTRY)) as {
  render: (location: string) => string
}

const DIST = join(import.meta.dirname, '..', 'dist')

const SITE_URL = 'https://michel.github.io/soulseek-rs/'

const escapeAttr = (value: string): string =>
  value.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;')

/** Replaces the content of one `<meta>` matched by attribute and value. */
const setMeta = (html: string, attr: string, name: string, value: string): string =>
  html.replace(
    new RegExp(`(<meta\\s+${attr}="${name}"\\s+content=")[\\s\\S]*?(")`),
    `$1${escapeAttr(value)}$2`,
  )

/**
 * Swaps every per-page tag in the template for the route's own.
 *
 * All five are listed here together on purpose: og:title and og:url used to be
 * left alone, so each prerendered page advertised the site title and the
 * homepage URL to anything reading Open Graph.
 */
const applyMeta = (html: string, meta: RouteMeta): string => {
  const canonical = meta.path === '/' ? SITE_URL : `${SITE_URL}${meta.path.slice(1)}/`
  let out = html.replace(
    /<title>[\s\S]*?<\/title>/,
    `<title>${escapeAttr(meta.title)}</title>`,
  )
  out = setMeta(out, 'name', 'description', meta.description)
  out = setMeta(out, 'property', 'og:title', meta.title)
  out = setMeta(out, 'property', 'og:description', meta.description)
  out = setMeta(out, 'property', 'og:url', canonical)
  return out.replace(
    /(<link\s+rel="canonical"\s+href=")[\s\S]*?(")/,
    `$1${escapeAttr(canonical)}$2`,
  )
}

const writePage = async (
  template: string,
  meta: RouteMeta,
  outPath: string,
  location: string,
): Promise<void> => {
  const html = applyMeta(template, meta).replace(
    '<div id="root"></div>',
    `<div id="root">${render(location)}</div>`,
  )
  const target = join(DIST, outPath)
  await mkdir(dirname(target), { recursive: true })
  await writeFile(target, html, 'utf8')
  console.log(`  ${outPath}`)
}

const template = await readFile(join(DIST, 'index.html'), 'utf8')

console.log('prerendering:')
for (const route of ROUTES) {
  const outPath = route.path === '/' ? 'index.html' : `${route.path.slice(1)}/index.html`
  await writePage(template, route, outPath, route.path)
}

// GitHub Pages serves this whenever no file matches, which after the loop
// above is only ever a genuinely unknown path.
await writePage(template, NOT_FOUND_META, '404.html', '/__not_found__')

console.log(`done — ${String(ROUTES.length + 1)} pages`)
