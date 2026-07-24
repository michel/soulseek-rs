import { Button } from '@/components/ui/button'
import { FeatureCard } from '@/components/ui/feature-card'
import { Code } from '@/components/ui/inline-code'
import { Cols, PageHead, Prose, Section, SectionHead } from '@/components/ui/layout'
import { Callout } from '@/components/ui/panel'
import { Terminal, type TermLine } from '@/components/ui/terminal'
import { LINKS } from '@/lib/links'

const LIB_SRC = `use soulseek_rs::Client;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Client::new("username", "password");
    client.connect();
    client.login()?;

    let results = client.search("public domain field recordings", Duration::from_secs(10))?;
    if let Some(r) = results.iter().find(|r| !r.files.is_empty()) {
        let f = &r.files[0];
        client.download(f.name.clone(), f.username.clone(), f.size, "~/Downloads".to_string())?;
    }
    Ok(())
}`

// LIB_SRC carries no comment lines, so every line is plain code.
const LIB_LINES: TermLine[] = LIB_SRC.split('\n').map((text) => ({
  t: 'code' as const,
  text,
}))

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
    note: 'macOS deliberately follows the XDG layout rather than Application Support, so config sits beside your other dotfile config.',
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

interface StepProps {
  n: number
  title: string
  children: React.ReactNode
}

const Step = ({ n, title, children }: StepProps) => (
  <div className="grid grid-cols-1 gap-3 sm:grid-cols-[auto_1fr] sm:gap-5">
    <div className="flex size-[34px] items-center justify-center rounded-md border border-line text-sm text-accent">
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

export const Install = () => {
  return (
    <>
      <PageHead eyebrow="install" title="Three ways in.">
        One command if you have Rust. Build from source if you&rsquo;d rather. Or depend on
        the library and write your own client.
      </PageHead>

      <Section>
        <div className="flex flex-col gap-9">
          <Step n={1} title="Install the client">
            <p className="text-secondary">
              Needs a Rust toolchain, available from{' '}
              <a
                href={LINKS.rustup}
                target="_blank"
                rel="noopener noreferrer"
                className="text-link hover:text-link-hover"
              >
                rustup.rs
              </a>
              . Then:
            </p>
            <Terminal
              lines={[
                { t: 'cmd', text: 'cargo install soulseek-rs' },
                { t: 'cmd', text: 'soulseek-rs' },
              ]}
            />
            <p className="text-[13px] text-muted">
              Cargo puts the binary on your PATH. Run <Code>soulseek-rs</Code> and the TUI
              opens.
            </p>
            <div className="flex flex-wrap gap-2 sm:gap-3">
              <Button href={LINKS.releases}>Prebuilt binaries ↗</Button>
            </div>
            <p className="text-[13px] text-muted">
              No Rust toolchain? Grab a prebuilt archive for your platform from the{' '}
              <a
                href={LINKS.releases}
                target="_blank"
                rel="noopener noreferrer"
                className="text-link hover:text-link-hover"
              >
                releases page
              </a>
              , unpack it, and put the binary somewhere on your PATH.
            </p>
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
                { t: 'code', text: 'soulseek-rs-lib = "6.0.0"' },
              ]}
              copy={'[dependencies]\nsoulseek-rs-lib = "6.0.0"'}
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
              . Note: v6 moved sharing from one directory to a list. See the{' '}
              <a
                href={LINKS.changelog}
                target="_blank"
                rel="noopener noreferrer"
                className="text-link hover:text-link-hover"
              >
                changelog
              </a>{' '}
              if you&rsquo;re upgrading from 5.x.
            </p>
          </Step>
        </div>
      </Section>

      <Section>
        <SectionHead eyebrow="being reachable" title="Let peers connect back.">
          Browsing and downloading are peer-to-peer, so at least one side has to accept an
          incoming connection.
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
              The port is your <Code>--listener-port</Code> (env <Code>LISTENER_PORT</Code>,
              default <Code>2234</Code>); it&rsquo;s renewed automatically and removed on
              exit. Pass <Code>--disable-listener</Code> to turn it off. Check your own
              network without launching the client:
            </p>
            <div className="mt-4">
              <Terminal lines={[{ t: 'cmd', text: 'soulseek-rs portmap' }]} />
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
              id={platform.id}
              className="scroll-mt-20 rounded-md border border-hairline bg-panel p-[18px] sm:p-[22px]"
            >
              <h3 className="mb-4 text-heading leading-[var(--text-heading--line-height)] font-medium">
                {platform.name}
              </h3>
              <dl className="grid grid-cols-1 gap-x-3.5 gap-y-0.5 sm:grid-cols-[auto_1fr] sm:gap-y-2">
                {(
                  [
                    ['Password', platform.credentialStore, false],
                    ['Config', platform.config, true],
                    ['State', platform.state, true],
                    ['Downloads', platform.downloads, true],
                  ] as const
                ).map(([label, value, mono]) => (
                  <div key={label} className="contents">
                    <dt className="pt-2 text-[11px] uppercase tracking-[var(--tracking-label)] text-secondary sm:pt-0.5">
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
            Your password goes to the OS keychain, never a plain-text file. If you would
            rather it never be stored at all, have <code>password_cmd</code> shell out to
            your own password manager and print it on demand.
          </FeatureCard>
          <FeatureCard title="Settings and state">
            <code>config.toml</code> is layered: flags beat environment variables, which
            beat the file, which beats the defaults. State (searches, downloads, rooms) is
            versioned JSON and restores on restart; anything unreadable is set aside as{' '}
            <code>.bak</code> instead of being overwritten.
          </FeatureCard>
        </Cols>
      </Section>

      <Section band>
        <SectionHead eyebrow="uninstall" title="Removing it completely.">
          Three things exist on disk: the binary, the config, and the state. Nothing else is
          written, and there is no telemetry to opt out of.
        </SectionHead>
        <Cols start>
          <Terminal
            label="macOS · Linux"
            lines={[
              { t: 'cmd', text: 'cargo uninstall soulseek-rs' },
              { t: 'cm', text: '# config, state and any cached shares' },
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
          <Callout title="two things it will not touch">
            <p>
              Your downloads and your shared directories are yours: they stay exactly where
              they are. On macOS, delete the keychain entry for <code>soulseek-rs</code> in
              Keychain Access if you saved a password.
            </p>
          </Callout>
        </div>
      </Section>
    </>
  )
}
