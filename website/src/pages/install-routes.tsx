import { ExtLink } from '@/components/ui/ext-link'
import { Code } from '@/components/ui/inline-code'
import { OsIcon, type OsName } from '@/components/ui/os-icon'
import { Terminal, type TermLine } from '@/components/ui/terminal'
import { LINKS, NIGHTLY_DOWNLOAD, STABLE_DOWNLOAD } from '@/lib/links'

interface InstallRoute {
  id: string
  os: OsName
  best: string
  lines: readonly TermLine[]
  alt: React.ReactNode
}

// aislop-ignore-file code-quality/duplicate-block -- data tables: the INSTALLS and LINUX_ROUTES records repeat the field names their interfaces require. The rule matches punctuation, not logic.
const INSTALLS: readonly InstallRoute[] = [
  {
    id: 'macos',
    os: 'macOS',
    best: 'Homebrew',
    lines: [
      { t: 'cmd', text: 'brew install michel/tap/soulseek-rs' },
      { t: 'cmd', text: 'soulseek-rs' },
    ],
    alt: (
      <>
        A prebuilt binary for Apple silicon and Intel, no Rust toolchain needed. With a
        toolchain, <Code>cargo install --locked soulseek-rs</Code> builds from crates.io instead.
      </>
    ),
  },
  {
    id: 'linux',
    os: 'Linux',
    best: 'your package manager',
    lines: [
      { t: 'cm', text: '# Debian and Ubuntu; the others are below' },
      { t: 'cmd', text: `curl -fsSLO ${STABLE_DOWNLOAD}/soulseek-rs-amd64.deb` },
      { t: 'cmd', text: 'sudo apt install ./soulseek-rs-amd64.deb' },
      { t: 'cmd', text: 'soulseek-rs' },
    ],
    alt: (
      <>
        Fedora, Arch, Nix, Homebrew and cargo are{' '}
        <a href="#linux-packages" className="text-link hover:text-link-hover">
          listed below
        </a>
        , each with a nightly. Without any of them, the{' '}
        <ExtLink href={LINKS.releases}>releases page</ExtLink> has static musl archives for x86-64
        and arm64 that run on any distribution.
      </>
    ),
  },
  {
    id: 'windows',
    os: 'Windows',
    best: 'cargo or a prebuilt .exe',
    lines: [
      { t: 'cmd', text: 'cargo install --locked soulseek-rs' },
      { t: 'cmd', text: 'soulseek-rs.exe' },
    ],
    alt: (
      <>
        Cargo needs the MSVC build tools. To skip both, take the{' '}
        <code>pc-windows-msvc</code> zip from the{' '}
        <ExtLink href={LINKS.releases}>releases page</ExtLink>, unpack it, and put{' '}
        <code>soulseek-rs.exe</code> on your PATH.
      </>
    ),
  },
]

interface LinuxRoute {
  id: string
  name: string
  stable: readonly TermLine[]
  nightly: readonly TermLine[]
  note: React.ReactNode
}

const LINUX_ROUTES: readonly LinuxRoute[] = [
  {
    id: 'deb',
    name: 'Debian, Ubuntu, Mint',
    stable: [
      { t: 'cmd', text: `curl -fsSLO ${STABLE_DOWNLOAD}/soulseek-rs-amd64.deb` },
      { t: 'cmd', text: 'sudo apt install ./soulseek-rs-amd64.deb' },
    ],
    nightly: [
      { t: 'cmd', text: `curl -fsSLO ${NIGHTLY_DOWNLOAD}/soulseek-rs-nightly-amd64.deb` },
      { t: 'cmd', text: 'sudo apt install ./soulseek-rs-nightly-amd64.deb' },
    ],
    note: (
      <>
        On ARM, <code>arm64</code> takes the place of <code>amd64</code>. There is no apt
        repository behind this, so an update means installing the newer file over the old one.
        A nightly sorts above the release it follows, so going back to stable is a downgrade,
        and apt asks before doing it.
      </>
    ),
  },
  {
    id: 'rpm',
    name: 'Fedora, RHEL',
    stable: [{ t: 'cmd', text: `sudo dnf install ${STABLE_DOWNLOAD}/soulseek-rs-x86_64.rpm` }],
    nightly: [
      { t: 'cmd', text: `sudo dnf install ${NIGHTLY_DOWNLOAD}/soulseek-rs-nightly-x86_64.rpm` },
    ],
    note: (
      <>
        On ARM, <code>aarch64</code> takes the place of <code>x86_64</code>. Run the same
        command again to update. dnf fetches the file each time, so a newer nightly at the same
        address is picked up. An older file downgrades without asking.
      </>
    ),
  },
  {
    id: 'aur',
    name: 'Arch, from the AUR',
    stable: [{ t: 'cmd', text: 'yay -S soulseek-rs-bin' }],
    nightly: [{ t: 'cmd', text: 'yay -S soulseek-rs-git' }],
    note: (
      <>
        <code>soulseek-rs-bin</code> installs the release binary. <code>soulseek-rs-git</code>{' '}
        builds <code>develop</code> from source, so it pulls in a Rust toolchain, and reports its
        package version, such as <code>19.0.0.r27.gdeb846b</code>.
      </>
    ),
  },
  {
    id: 'nix',
    name: 'Nix',
    stable: [{ t: 'cmd', text: 'nix run github:michel/soulseek-rs/master' }],
    nightly: [{ t: 'cmd', text: 'nix run github:michel/soulseek-rs/develop' }],
    note: (
      <>
        Both build from source. With no branch at all, Nix takes the repository&rsquo;s
        default one, and here that is <code>develop</code>.
      </>
    ),
  },
  {
    id: 'brew',
    name: 'Homebrew',
    stable: [{ t: 'cmd', text: 'brew install michel/tap/soulseek-rs' }],
    nightly: [{ t: 'cmd', text: 'brew install --HEAD michel/tap/soulseek-rs' }],
    note: (
      <>
        Homebrew installs under <code>/home/linuxbrew</code>, outside the image that a system
        update replaces on Bazzite and Silverblue. <code>--HEAD</code> compiles{' '}
        <code>develop</code> with Homebrew&rsquo;s own Rust.
      </>
    ),
  },
  {
    id: 'cargo',
    name: 'cargo',
    stable: [{ t: 'cmd', text: 'cargo install --locked soulseek-rs' }],
    nightly: [
      {
        t: 'cmd',
        text: 'cargo install --locked --git https://github.com/michel/soulseek-rs --branch develop soulseek-rs',
      },
    ],
    note: (
      <>
        Needs Rust 1.91 or newer. Debian 13 and Ubuntu 24.04 default to an older one, so take
        the toolchain from <ExtLink href={LINKS.rustup}>rustup.rs</ExtLink>.{' '}
        <code>cargo binstall soulseek-rs</code> fetches the release binary and compiles nothing.
      </>
    ),
  },
]

export const InstallCards = () => (
  <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
    {INSTALLS.map((route) => (
      <div
        key={route.id}
        id={route.id}
        className="flex scroll-mt-20 flex-col gap-3.5 rounded-md border border-hairline bg-panel p-[18px] sm:p-[22px]"
      >
        <div className="flex items-center gap-2.5">
          <OsIcon name={route.os} />
          <h3 className="text-heading leading-[var(--text-heading--line-height)] font-medium">
            {route.os}
          </h3>
        </div>
        <span className="text-[11px] uppercase tracking-[var(--tracking-label)] text-secondary">
          {route.best}
        </span>
        <Terminal lines={route.lines} wrap />
        <p className="text-[12.5px] leading-5 text-secondary [&_code]:font-mono [&_code]:text-primary">
          {route.alt}
        </p>
      </div>
    ))}
  </div>
)

export const LinuxRoutes = () => (
  <div id="linux-packages" className="flex scroll-mt-20 flex-col gap-5">
    <h3 className="text-heading leading-[var(--text-heading--line-height)] font-medium">
      Linux, by distribution
    </h3>
    <p className="text-secondary">
      Stable is the latest release, or for Nix the <Code>master</Code> branch it was cut
      from. Nightly is the tip of <Code>develop</Code>, rebuilt every day. A release download
      prints the plain number, <Code>19.0.0</Code>. Any Nix build, and a nightly download,
      reads <Code>19.0.0+git202609141144.deb846b</Code>: the last release, the
      commit&rsquo;s time in UTC, and the commit. A Homebrew <Code>--HEAD</Code> build
      reports <Code>HEAD-deb846b</Code>, and a <Code>cargo install</Code> from git sets no
      version, so it prints <Code>19.0.0</Code> too. The .deb and .rpm
      carry the man page and completions for bash, zsh and fish.
    </p>
    {LINUX_ROUTES.map((route) => (
      <div key={route.id} className="flex flex-col gap-2.5">
        <span className="text-[11px] uppercase tracking-[var(--tracking-label)] text-secondary">
          {route.name}
        </span>
        <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
          <Terminal label="stable" lines={route.stable} wrap />
          <Terminal label="nightly" lines={route.nightly} wrap />
        </div>
        <p className="text-[12.5px] leading-5 text-secondary [&_code]:font-mono [&_code]:text-primary">
          {route.note}
        </p>
      </div>
    ))}
  </div>
)
