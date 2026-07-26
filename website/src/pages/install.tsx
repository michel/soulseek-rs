import { Button } from '@/components/ui/button'
import { FeatureCard } from '@/components/ui/feature-card'
import { Code } from '@/components/ui/inline-code'
import { Cols, PageHead, Prose, Section, SectionHead } from '@/components/ui/layout'
import { OsIcon, type OsName } from '@/components/ui/os-icon'
import { Callout } from '@/components/ui/panel'
import { Terminal, type TermLine } from '@/components/ui/terminal'
import { LINKS } from '@/lib/links'

const LIB_SRC = `use soulseek_rs::Client;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Client::new("username", "password");
    client.connect()?;
    client.login()?;

    let results = client.search("public domain field recordings", Duration::from_secs(10))?;
    if let Some(r) = results.iter().find(|r| !r.files.is_empty()) {
        let f = &r.files[0];
        client.download(f.name.clone(), f.username.clone(), f.size, "~/Downloads".to_string())?;
    }
    Ok(())
}`

const LIB_LINES: TermLine[] = LIB_SRC.split('\n').map((text) => ({
  t: 'code' as const,
  text,
}))

const ReleasesLink = ({ children }: { children: React.ReactNode }) => (
  <a
    href={LINKS.releases}
    target="_blank"
    rel="noopener noreferrer"
    className="text-link hover:text-link-hover"
  >
    {children}
  </a>
)

interface InstallRoute {
  id: string
  os: OsName
  best: string
  lines: readonly TermLine[]
  alt: React.ReactNode
}

// aislop-ignore-file code-quality/duplicate-block -- three findings here, all data tables: the INSTALLS records repeat the field names InstallRoute requires, and the two uninstall `lines` arrays repeat the `{ t, text }` shape around different commands. The rule matches punctuation, not logic.
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
        toolchain, <Code>cargo install soulseek-rs</Code> builds from crates.io instead.
      </>
    ),
  },
  {
    id: 'linux',
    os: 'Linux',
    best: 'cargo',
    lines: [
      { t: 'cmd', text: 'cargo install soulseek-rs' },
      { t: 'cmd', text: 'soulseek-rs' },
    ],
    alt: (
      <>
        Homebrew on Linux works the same as on macOS. Without either, the{' '}
        <ReleasesLink>releases page</ReleasesLink> has static musl archives for x86-64 and
        arm64 that run on any distribution.
      </>
    ),
  },
  {
    id: 'windows',
    os: 'Windows',
    best: 'cargo or a prebuilt .exe',
    lines: [
      { t: 'cmd', text: 'cargo install soulseek-rs' },
      { t: 'cmd', text: 'soulseek-rs.exe' },
    ],
    alt: (
      <>
        Cargo needs the MSVC build tools. To skip both, take the{' '}
        <code>pc-windows-msvc</code> zip from the{' '}
        <ReleasesLink>releases page</ReleasesLink>, unpack it, and put{' '}
        <code>soulseek-rs.exe</code> on your PATH.
      </>
    ),
  },
]

interface PlatformNote {
  id: string
  name: string
  credentialStore: string
  config: string
  state: string
  downloads: string
  note: string
}

const PLATFORMS: readonly PlatformNote[] = [
  {
    id: 'macos',
    name: 'macOS',
    credentialStore: 'Apple Keychain',
    config: '~/.config/soulseek-rs/config.toml',
    state: '~/.local/share/soulseek-rs/state',
    downloads: '~/Downloads/Soulseek',
    note: 'Follows the XDG layout rather than Application Support.',
  },
  {
    id: 'linux',
    name: 'Linux',
    credentialStore: 'Secret Service (pure-Rust backend, no libdbus needed)',
    config: '~/.config/soulseek-rs/config.toml',
    state: '~/.local/share/soulseek-rs/state',
    downloads: '~/Downloads/Soulseek',
    note: 'Honours XDG_CONFIG_HOME and XDG_DATA_HOME when they are set.',
  },
  {
    id: 'windows',
    name: 'Windows',
    credentialStore: 'Windows Credential Manager',
    config: '%APPDATA%\\soulseek-rs\\config\\config.toml',
    state: '%APPDATA%\\soulseek-rs\\data\\state',
    downloads: '%USERPROFILE%\\Downloads\\Soulseek',
    note: 'Uses the standard known-folder locations for roaming app data.',
  },
]

interface Setting {
  key: string
  fallback: string
  body: React.ReactNode
}

const SETTINGS: readonly Setting[] = [
  {
    key: 'username',
    fallback: 'unset',
    body: <>Your Soulseek account name. The TUI writes it here after a first login.</>,
  },
  {
    key: 'server',
    fallback: 'server.slsknet.org:2416',
    body: <>The server to log in to, as host:port.</>,
  },
  {
    key: 'listener_port',
    fallback: '2234',
    body: (
      <>
        The TCP port peers connect back on. Forward it, or let <code>portmap</code> ask
        your router.
      </>
    ),
  },
  {
    key: 'disable_listener',
    fallback: 'false',
    body: (
      <>
        <code>true</code> refuses incoming peer connections. You can still download from
        reachable peers, but firewalled ones can no longer reach you.
      </>
    ),
  },
  {
    key: 'download_dir',
    fallback: 'the Downloads path above',
    body: <>Where finished transfers land.</>,
  },
  {
    key: 'shared_dirs',
    fallback: 'the download folder',
    body: (
      <>
        Folders offered to the network. An empty list shares nothing.{' '}
        <code>shares add</code> writes this; the older <code>shared_dir</code> spelling
        still reads.
      </>
    ),
  },
  {
    key: 'max_concurrent_downloads',
    fallback: '5',
    body: <>Transfers allowed to run at once. The rest queue.</>,
  },
  {
    key: 'search_timeout',
    fallback: '10',
    body: (
      <>
        Seconds a search collects replies before reporting. Results trickle in from
        separate peers, so a short window finds less.
      </>
    ),
  },
  {
    key: 'password_cmd',
    fallback: 'unset',
    body: (
      <>
        A shell command whose stdout is the password, used when the keychain has none.
        The password itself never belongs in this file.
      </>
    ),
  },
]

const CONFIG_EXAMPLE: TermLine[] = [
  'username = "your-name"',
  'download_dir = "~/Music/Soulseek"',
  'shared_dirs = ["~/Music/Soulseek", "~/Music/Library"]',
  'max_concurrent_downloads = 3',
  'search_timeout = 15',
  'listener_port = 2234',
  '',
  '# never store the password itself; shell out for it instead',
  'password_cmd = "pass show soulseek"',
].map((text) => ({ t: 'code' as const, text }))

interface StepProps {
  n: number
  title: string
  children: React.ReactNode
}

const Step = ({ n, title, children }: StepProps) => (
  <div className="grid grid-cols-1 gap-3 sm:grid-cols-[auto_1fr] sm:gap-5">
    <div className="flex size-[34px] items-center justify-center rounded-md border border-line text-sm text-accent-text">
      {n}
    </div>
    <div className="flex min-w-0 flex-col gap-3">
      <h2 className="text-heading leading-[var(--text-heading--line-height)] font-medium">
        {title}
      </h2>
      {children}
    </div>
  </div>
)

const Steps = () => (
  <Section>
    <div className="flex flex-col gap-9">
      <Step n={1} title="Install the client">
        <p className="text-secondary">
          One command per platform. Cargo needs a Rust toolchain from{' '}
          <a
            href={LINKS.rustup}
            target="_blank"
            rel="noopener noreferrer"
            className="text-link hover:text-link-hover"
          >
            rustup.rs
          </a>
          ; the other routes don&rsquo;t.
        </p>
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
        <p className="text-[13px] text-muted">
          Every route puts the same binary on your PATH. Run <Code>soulseek-rs</Code> in
          a terminal and the TUI opens, in a script or a pipe it wants a subcommand
          instead.
        </p>
        <p className="text-[13px] text-muted">
          <Code>soulseek-rs completions install</Code> then adds tab completion to
          whichever of bash, zsh and fish this machine uses &mdash; generated from the
          same definition the binary parses with, so it never drifts from{' '}
          <Code>--help</Code>. Open a new shell to pick it up.
        </p>
        <div className="flex flex-wrap gap-2 sm:gap-3">
          <Button href={LINKS.releases}>Prebuilt binaries</Button>
        </div>
      </Step>

      <Step n={2} title="Build from source">
        <p className="text-secondary">
          Clone the workspace and build a release binary.
        </p>
        <Terminal
          lines={[
            { t: 'cmd', text: 'git clone https://github.com/michel/soulseek-rs.git' },
            { t: 'cmd', text: 'cd soulseek-rs' },
            { t: 'cmd', text: 'cargo build --release' },
          ]}
        />
        <p className="text-[13px] text-muted">
          You&rsquo;ll find the binary at <Code>target/release/soulseek-rs</Code>.
          Prebuilt archives for tagged releases are on the{' '}
          <a
            href={LINKS.releases}
            target="_blank"
            rel="noopener noreferrer"
            className="text-link hover:text-link-hover"
          >
            releases page
          </a>
          .
        </p>
      </Step>

      <Step n={3} title="Build on the library">
        <p className="text-secondary">
          Write your own client or bot on <Code>soulseek-rs-lib</Code>: the protocol
          implementation, separate from the TUI.
        </p>
        <Terminal
          label="Cargo.toml"
          lines={[
            { t: 'cm', text: '[dependencies]' },
            { t: 'code', text: 'soulseek-rs-lib = "8"' },
          ]}
          copy={'[dependencies]\nsoulseek-rs-lib = "8"'}
        />
        <Terminal label="src/main.rs" lines={LIB_LINES} copy={LIB_SRC} />
        <p className="text-[13px] text-muted">
          Full API reference on{' '}
          <a
            href={LINKS.docsrs}
            target="_blank"
            rel="noopener noreferrer"
            className="text-link hover:text-link-hover"
          >
            docs.rs
          </a>
          . The lib API changes between majors, so check the{' '}
          <a
            href={LINKS.changelog}
            target="_blank"
            rel="noopener noreferrer"
            className="text-link hover:text-link-hover"
          >
            changelog
          </a>{' '}
          when you upgrade.
        </p>
      </Step>
    </div>
  </Section>
)
const Reachable = () => (
  <Section>
    <SectionHead eyebrow="being reachable" title="Let peers connect back.">
      Browsing and downloading are peer-to-peer, so at least one side has to accept an
      incoming connection. <Code>serve</Code> refuses to start with the listener off.
    </SectionHead>
    <Cols start>
      <Prose>
        <p>
          With the listener on (the default), soulseek-rs tries to open its listen port
          on your router via{' '}
          <b className="font-medium text-primary">UPnP-IGD</b> and{' '}
          <b className="font-medium text-primary">NAT-PMP</b>. It&rsquo;s best-effort:
          if your router has those disabled it&rsquo;s a no-op, and you forward the port
          yourself.
        </p>
        <p>
          The port is your <Code>--listener-port</Code> (env{' '}
          <Code>SOULSEEK_LISTENER_PORT</Code>, default <Code>2234</Code>); soulseek-rs
          renews it while it runs and removes it on exit. Pass{' '}
          <Code>--no-listener</Code> to turn it off. Check your own network without
          launching the client:
        </p>
        <div className="mt-4">
          <Terminal
            lines={[
              { t: 'cmd', text: 'soulseek-rs portmap' },
              { t: 'cm', text: "# exits 0 when it works, 4 when it doesn't" },
            ]}
          />
        </div>
      </Prose>
      <Callout tone="warn" title="honest about limits">
        <p>
          If both you and a peer are behind routers with no forwarded port, browsing
          that peer can&rsquo;t work, that&rsquo;s a fundamental Soulseek/peer-to-peer
          limitation, not a bug. Forward <Code>--listener-port</Code>, or let{' '}
          <Code>portmap</Code> try UPnP.
        </p>
      </Callout>
    </Cols>
  </Section>
)
const Platforms = () => (
  <Section id="platforms">
    <SectionHead eyebrow="platform notes" title="Where things live.">
      Sensible defaults per OS, all overridable. Set <Code>SOULSEEK_CONFIG_DIR</Code> or{' '}
      <Code>SOULSEEK_STATE_DIR</Code> to relocate either one, which also makes a
      portable install possible.
    </SectionHead>
    <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
      {PLATFORMS.map((platform) => (
        <div
          key={platform.id}
          id={`${platform.id}-paths`}
          className="scroll-mt-20 rounded-md border border-hairline bg-panel p-[18px] sm:p-[22px]"
        >
          <h3 className="mb-4 text-heading leading-[var(--text-heading--line-height)] font-medium">
            {platform.name}
          </h3>
          <dl className="grid grid-cols-1 gap-x-3.5 gap-y-0.5 sm:grid-cols-[auto_1fr] sm:gap-y-2 lg:grid-cols-1 lg:gap-y-0.5">
            {(
              [
                ['Password', platform.credentialStore, false],
                ['Config', platform.config, true],
                ['State', platform.state, true],
                ['Downloads', platform.downloads, true],
              ] as const
            ).map(([label, value, mono]) => (
              <div key={label} className="contents">
                <dt className="pt-2 text-[11px] uppercase tracking-[var(--tracking-label)] text-secondary sm:pt-0.5 lg:pt-2">
                  {label}
                </dt>
                <dd className="min-w-0 text-[12.5px] leading-[19px] text-primary">
                  {mono ? (
                    <code className="font-mono text-xs break-all">{value}</code>
                  ) : (
                    value
                  )}
                </dd>
              </div>
            ))}
          </dl>
          <p className="mt-4 border-t border-hairline pt-3.5 text-[12.5px] leading-5 text-secondary">
            {platform.note}
          </p>
        </div>
      ))}
    </div>

    <Cols start className="mt-7">
      <FeatureCard title="Secrets">
        Your password goes to the OS keychain, never a plain-text file. Point{' '}
        <code>password_cmd</code> at your own password manager and it is never stored
        at all. For CI and containers,{' '}
        <code>--password-stdin</code> reads it off a pipe so it never reaches{' '}
        <code>ps</code> or the environment.
      </FeatureCard>
      <FeatureCard title="Settings and state">
        <code>config.toml</code> is layered: flags beat environment variables, which
        beat the file, which beats the defaults. State (searches, downloads, rooms) is
        versioned JSON and restores on restart; soulseek-rs sets anything unreadable
        aside as <code>.bak</code> rather than overwriting it.
      </FeatureCard>
    </Cols>
  </Section>
)
const Config = () => (
  <Section id="config">
    <SectionHead eyebrow="config.toml" title="Nine settings, all optional.">
      Every one has a working default, so an empty file, or no file at all, is valid.{' '}
      <Code>config list</Code> prints the effective value of each.
    </SectionHead>
    <Cols start>
      <div className="flex flex-col">
        {SETTINGS.map((setting) => (
          <div
            key={setting.key}
            className="grid grid-cols-1 gap-x-4 border-t border-hairline py-3.5 first:border-t-0 first:pt-0 sm:grid-cols-[minmax(0,15rem)_1fr]"
          >
            <div className="flex min-w-0 flex-col gap-1">
              <code className="font-mono text-[12.5px] break-all text-primary">
                {setting.key}
              </code>
              <span className="text-[11px] uppercase tracking-[var(--tracking-label)] text-secondary">
                {setting.fallback}
              </span>
            </div>
            <p className="mt-1.5 text-[13px] leading-[21px] text-pretty text-secondary sm:mt-0 [&_code]:font-mono [&_code]:text-primary">
              {setting.body}
            </p>
          </div>
        ))}
      </div>
      <div className="flex flex-col gap-4">
        <Terminal label="config.toml" lines={CONFIG_EXAMPLE} />
        <Callout title="editing it without an editor">
          <p>
            <Code>config path</Code> prints the file in use, <Code>config get</Code>{' '}
            reads one setting and <Code>config set</Code> writes one. It rejects
            unknown keys, so a typo fails instead of doing nothing.{' '}
            <Code>--no-config</Code> ignores the file entirely and{' '}
            <Code>--config FILE</Code> points at another one.
          </p>
        </Callout>
      </div>
    </Cols>
  </Section>
)
const Uninstall = () => (
  <Section band>
    <SectionHead eyebrow="uninstall" title="Removing it completely.">
      Three things exist on disk: the binary, the config, and the state &mdash; plus
      the completion scripts and the agent skill, if you asked for those. Nothing else,
      unless you point <Code>--log-file</Code> somewhere. No telemetry to opt out of.
    </SectionHead>
    <Cols start>
      <Terminal
        label="macOS · Linux"
        lines={[
          { t: 'cm', text: '# while the binary is still there' },
          { t: 'cmd', text: 'soulseek-rs completions uninstall' },
          { t: 'cmd', text: 'soulseek-rs skills uninstall' },
          { t: 'cmd', text: 'cargo uninstall soulseek-rs' },
          { t: 'cm', text: '# config and state: searches, downloads, rooms, messages' },
          {
            t: 'cmd',
            text: 'rm -rf ~/.config/soulseek-rs ~/.local/share/soulseek-rs',
          },
          { t: 'cm', text: '# Linux: drop the saved password' },
          { t: 'cmd', text: 'secret-tool clear service soulseek-rs' },
        ]}
      />
      <Terminal
        label="Windows · PowerShell"
        lines={[
          { t: 'cmd', text: 'soulseek-rs skills uninstall' },
          { t: 'cmd', text: 'cargo uninstall soulseek-rs' },
          {
            t: 'cmd',
            text: 'Remove-Item -Recurse -Force $env:APPDATA\\soulseek-rs',
          },
          { t: 'cm', text: '# then remove the soulseek-rs entry from' },
          { t: 'cm', text: '# Credential Manager > Windows Credentials' },
        ]}
      />
    </Cols>
    <div className="mt-5">
      <Callout className="bg-raised" title="two things it will not touch">
        <p>
          Your downloads and your shared directories are yours: they stay where they
          are. On macOS, delete the keychain entry for <code>soulseek-rs</code> in
          Keychain Access if you saved a password.
        </p>
      </Callout>
    </div>
  </Section>
)
export const Install = () => (
  <>
    <PageHead eyebrow="install" title="Three ways in.">
      One command with Homebrew or cargo. Build from source if you&rsquo;d rather. Or depend
      on the library and write your own client.
    </PageHead>
    <Steps />
    <Reachable />
    <Platforms />
    <Config />
    <Uninstall />
  </>
)
