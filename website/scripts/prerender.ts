import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'

import { NOT_FOUND_META, ROUTES, type RouteMeta } from '../src/lib/routes.ts'

const SSR_ENTRY = '../dist-ssr/entry-server.js'
const { render } = (await import(SSR_ENTRY)) as {
  render: (location: string) => string
}

const DIST = join(import.meta.dirname, '..', 'dist')

const SITE_URL = 'https://re-invention.nl/soulseek-rs/'

const escapeAttr = (value: string): string =>
  value.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;')

const setMeta = (html: string, attr: string, name: string, value: string): string =>
  html.replace(
    new RegExp(`(<meta\\s+${attr}="${name}"\\s+content=")[\\s\\S]*?(")`),
    `$1${escapeAttr(value)}$2`,
  )

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

await writePage(template, NOT_FOUND_META, '404.html', '/__not_found__')

console.log(`done — ${String(ROUTES.length + 1)} pages`)
