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
  seeleseek: 'https://seeleseek.net',
  seakarr: 'https://github.com/binhex/seakarr',
  player: 'https://github.com/verncat/player',
  staccato: 'https://github.com/bladew0rks/staccato',
  minorVersions: 'https://github.com/michel/soulseek-rs/issues/12',
} as const

export const VERSION = __APP_VERSION__

export const SITE_URL = 'https://re-invention.nl/soulseek-rs/'

export const INSTALL_CMD = `curl -fsSL ${SITE_URL}install.sh | sh`
