/** Every off-site destination, in one place. */
export const LINKS = {
  gh: 'https://github.com/michel/soulseek-rs',
  issues: 'https://github.com/michel/soulseek-rs/issues',
  releases: 'https://github.com/michel/soulseek-rs/releases',
  changelog: 'https://github.com/michel/soulseek-rs/blob/master/CHANGELOG.md',
  license: 'https://github.com/michel/soulseek-rs/blob/master/LICENSE',
  cratesClient: 'https://crates.io/crates/soulseek-rs',
  cratesLib: 'https://crates.io/crates/soulseek-rs-lib',
  docsrs: 'https://docs.rs/soulseek-rs-lib',
  rustup: 'https://rustup.rs',
  soulfind: 'https://github.com/soulfind-dev/soulfind',
  nicotine: 'https://nicotine-plus.org',
  slskd: 'https://github.com/slskd/slskd',
  slsknet: 'https://www.slsknet.org',
} as const

/** Injected from the workspace Cargo.toml at build time — see vite.config.ts. */
export const VERSION = __APP_VERSION__

/** Where the site is served from. Used for canonical and og:url. */
export const SITE_URL = 'https://re-invention.nl/soulseek-rs/'
