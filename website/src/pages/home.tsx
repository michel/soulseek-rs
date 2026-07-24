import { Link } from 'react-router'

import { Logo } from '@/components/brand/logo'
import { HeroTerminal } from '@/components/tui/hero-terminal'
import { Button } from '@/components/ui/button'
import { CommandPill } from '@/components/ui/command-pill'
import { FeatureCard } from '@/components/ui/feature-card'
import { Code } from '@/components/ui/inline-code'
import { Cols, Eyebrow, Prose, Section, SectionHead, Wrap } from '@/components/ui/layout'
import { OsIcon, type OsName } from '@/components/ui/os-icon'
import { Callout } from '@/components/ui/panel'
import { Terminal } from '@/components/ui/terminal'
import { LINKS } from '@/lib/links'
import { cn } from '@/lib/utils'

const PLATFORMS: readonly { name: OsName; note: string; anchor: string }[] = [
  { name: 'macOS', note: 'Apple Keychain', anchor: 'macos' },
  { name: 'Linux', note: 'Secret Service', anchor: 'linux' },
  { name: 'Windows', note: 'Credential Manager', anchor: 'windows' },
]

const FEATURES = [
  {
    glyph: '→',
    color: 'var(--accent-text)',
    title: 'Search & download',
    cmd: 'soulseek-rs "field recordings"',
    body: <>Search the network, pick results in the TUI, and queue downloads with pause, resume, and retry.</>,
  },
  {
    glyph: '↑',
    color: 'var(--status-success)',
    title: 'Sharing',
    cmd: 'soulseek-rs --shared-dir ~/Music',
    body: (
      <>
        Point <code>--shared-dir</code> at a directory and your files show up in searches;
        peers browse and download them.
      </>
    ),
  },
  {
    glyph: '›',
    color: 'var(--status-info)',
    title: 'Browse',
    cmd: 'soulseek-rs browse <user>',
    body: <>List any user&rsquo;s shared files and download straight from the tree.</>,
  },
  {
    glyph: '#',
    color: 'var(--status-warning)',
    title: 'Chat rooms',
    cmd: 'soulseek-rs rooms',
    body: <>List, join, and talk in public rooms, several open at once as tabs.</>,
  },
  {
    glyph: '@',
    color: 'var(--status-info)',
    title: 'Private messages',
    cmd: 'soulseek-rs message <user> "hi"',
    body: <>Send and receive PMs, with an inbox and an unread counter in the TUI.</>,
  },
  {
    glyph: '↔',
    color: 'var(--status-info)',
    title: 'Firewalled peers',
    cmd: 'soulseek-rs --listener-port 2234',
    body: (
      <>
        Downloads and browsing fall back to a server-brokered connection when a peer
        can&rsquo;t be reached directly.
      </>
    ),
  },
  {
    glyph: '↯',
    color: 'var(--status-warning)',
    title: 'Automatic port mapping',
    cmd: 'soulseek-rs portmap',
    body: (
      <>
        Opens your listen port via UPnP-IGD and NAT-PMP, with a <code>portmap</code>{' '}
        subcommand to test your router.
      </>
    ),
  },
  {
    glyph: '▮',
    color: 'var(--accent-text)',
    title: 'TUI and CLI',
    cmd: 'soulseek-rs',
    body: (
      <>
        A full terminal interface, plus scriptable subcommands: search, message, browse,
        rooms, chat, portmap.
      </>
    ),
  },
] as const

export const Home = () => {
  return (
    <>
      <section className="pt-5 pb-8 sm:pt-7 sm:pb-10">
        <Wrap>
          <div className="flex flex-col gap-[22px]">
            {/* Eyebrow and sub are desktop-only: on a phone the headline and
                the CTA should be all that stands between the top of the page
                and the terminal. */}
            <Eyebrow className="max-md:hidden">Soulseek client · terminal · Rust</Eyebrow>
            <div className="flex items-start gap-3.5 sm:gap-6 md:gap-11">
              <Logo size={172} className="mt-2 max-md:!h-28 max-sm:!h-16" />
              <h1 className="text-[29px] leading-[37px] font-bold tracking-[-0.01em] text-balance sm:text-[34px] sm:leading-[42px] md:text-display md:leading-[var(--text-display--line-height)]">
                A soulseek client for the terminal. Built for agents and people who live
                there.
              </h1>
            </div>
            <p className="text-lg leading-[29px] text-pretty text-secondary max-md:hidden">
              Search the network, share your files, browse someone&rsquo;s collection, join
              a room. It runs over ssh on the machine where your music already lives.
            </p>
            <div className="flex flex-wrap items-center justify-between gap-x-5 gap-y-3 sm:gap-y-4">
              <div className="flex flex-wrap items-center gap-2 max-md:w-full max-md:justify-center sm:gap-3">
                <Button href="/install" variant="accent">
                  Install
                </Button>
                <Button href="/docs">Read the docs</Button>
                <Button href={LINKS.gh}>GitHub</Button>
              </div>
              <CommandPill text="cargo install soulseek-rs" />
            </div>
          </div>

          <div className="mt-3.5 overflow-hidden">
            <HeroTerminal />
          </div>
        </Wrap>
      </section>

      <section className="border-t border-hairline py-6">
        <Wrap>
          {/*
            Three layouts, narrowest first: under 520px the labels drop and it
            becomes a centred row of icons; up to 820px each platform is a
            full-width stacked row; above that they sit inline, rule-separated.
          */}
          <div className="flex flex-wrap items-center gap-3.5 sm:gap-8">
            <Eyebrow className="max-[519px]:hidden">Runs on</Eyebrow>
            <ul className="flex w-full flex-nowrap justify-center min-[520px]:flex-wrap min-[520px]:justify-start sm:w-auto">
              {PLATFORMS.map((platform, i) => (
                <li
                  key={platform.name}
                  className={cn(
                    'flex w-auto border-hairline px-[18px] first:border-l-0 first:pl-0 not-first:border-l',
                    'min-[520px]:w-full min-[520px]:border-l-0 min-[520px]:px-0',
                    i === 0
                      ? 'sm:w-auto sm:pr-6.5'
                      : 'sm:w-auto sm:border-l sm:px-6.5',
                  )}
                >
                  <Link
                    to={`/install#${platform.anchor}`}
                    className="group flex min-h-11 w-auto items-center justify-center gap-2.5 border-hairline py-1.5 min-[520px]:w-full min-[520px]:justify-start min-[520px]:border-t min-[520px]:py-2.5 min-[520px]:first:border-t-0 sm:w-auto sm:border-t-0 sm:py-0"
                  >
                    <OsIcon name={platform.name} className="max-[519px]:size-[26px]" />
                    <span className="flex flex-col leading-[1.35] max-[519px]:hidden">
                      <b className="text-[13.5px] font-medium text-primary transition-colors group-hover:text-accent-text">
                        {platform.name}
                      </b>
                      <span className="text-[11.5px] text-secondary">{platform.note}</span>
                    </span>
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        </Wrap>
      </section>

      <Section id="about">
        <SectionHead
          eyebrow="what it is"
          title="A client for a network that runs on people."
        >
          No charts, no recommendations. You search, you find the person who has it, you
          download it from them.
        </SectionHead>
        <Cols>
          <Prose>
            <p>
              Soulseek is a peer-to-peer network from the early 2000s. People still use it
              to share niche music by hand: the album that exists in one person&rsquo;s
              folder and nowhere else.
            </p>
            <p>
              soulseek-rs speaks that protocol. It&rsquo;s a third-party client, not the
              official app, and not affiliated with the Soulseek project. It shares,
              browses, and does rooms and private messages, because a client that only
              downloads takes from the network without giving anything back.
            </p>
          </Prose>
          <div className="flex flex-col gap-4">
            <Callout title="build your own">
              <p>
                Reach for <Code>soulseek-rs-lib</Code> to build a custom client or an agent
                on the protocol directly.{' '}
                <Link to="/install" className="text-link hover:text-link-hover">
                  Use the library
                </Link>
              </p>
            </Callout>
            <Callout title="agents and automation">
              <p>
                Every command runs as a one-off from the shell: <Code>search</Code>,{' '}
                <Code>browse</Code>, <Code>rooms</Code>, <Code>chat</Code>,{' '}
                <Code>message</Code>, <Code>portmap</Code>. Point cron, a script, or an
                agent at them and read the output back. No daemon, no API to stand up.{' '}
                <Link to="/docs" className="text-link hover:text-link-hover">
                  See the commands
                </Link>
              </p>
            </Callout>
          </div>
        </Cols>
      </Section>

      <Section id="features">
        <SectionHead eyebrow="features" title="What it does.">
          Everything happens in the TUI, and every command also runs as a one-off from the
          shell, so the same client works for scripting, cron jobs, and agents driving it
          programmatically.
        </SectionHead>
        {/*
          Three up, not four, with search spanning two of them: it's the thing
          the client is for, and eight cards at identical weight said the
          opposite. Eight items plus that one double = nine cells, so the grid
          still comes out square.
        */}
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {FEATURES.map((feature, i) => (
            <FeatureCard
              key={feature.title}
              glyph={feature.glyph}
              color={feature.color}
              title={feature.title}
              cmd={feature.cmd}
              className={i === 0 ? 'lg:col-span-2' : ''}
            >
              {feature.body}
            </FeatureCard>
          ))}
        </div>
      </Section>

      <Section band>
        <Cols center>
          <div className="flex flex-col gap-3.5">
            <Eyebrow>get started</Eyebrow>
            <h2 className="text-[26px] leading-[34px] font-medium text-balance sm:text-title sm:leading-[var(--text-title--line-height)]">
              Install it, then press <span className="text-accent-text">/</span> to search.
            </h2>
            <p className="text-secondary">
              Needs a Rust toolchain. From there it&rsquo;s one command, then the TUI opens.
            </p>
            <div className="mt-1.5 flex flex-wrap gap-2 sm:gap-3">
              <Button href="/docs" variant="accent">
                Read the docs
              </Button>
              <Button href="/install">Install options</Button>
            </div>
          </div>
          <Terminal
            label="quick start"
            lines={[
              { t: 'cmd', text: 'cargo install soulseek-rs' },
              { t: 'cmd', text: 'soulseek-rs' },
            ]}
          />
        </Cols>
      </Section>
    </>
  )
}
