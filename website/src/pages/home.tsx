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
  { name: 'macOS', note: 'brew or cargo', anchor: 'macos' },
  { name: 'Linux', note: 'cargo install', anchor: 'linux' },
  { name: 'Windows', note: 'cargo or .exe', anchor: 'windows' },
]

// aislop-ignore-file code-quality/duplicate-block -- FEATURES is a data table; its entries repeat the field names the record shape requires, not logic. The rule matches the punctuation skeleton between entries. A factory would trade named fields for positional string args.
const FEATURES = [
  {
    glyph: '✦',
    color: 'var(--accent-text)',
    title: 'Agent native',
    cmd: 'soulseek-rs skills install',
    body: (
      <>
        Run it once and your agent knows the record shapes, the exit codes, and the
        filters: &ldquo;find the 1998 pressing, lossless only&rdquo; becomes a pipeline it
        writes itself. No MCP server, no wrapper. The skill ships in the binary and finds
        Claude Code and opencode on its own.
      </>
    ),
  },
  {
    glyph: '→',
    color: 'var(--accent-text)',
    title: 'Search & download',
    cmd: 'soulseek-rs get "field recordings"',
    body: (
      <>
        Queue downloads in the TUI with pause, resume, and retry, or let <code>get</code>{' '}
        search, pick the best, and download in one command.
      </>
    ),
  },
  {
    glyph: '↑',
    color: 'var(--status-success)',
    title: 'Sharing',
    cmd: 'soulseek-rs shares add ~/Music',
    body: (
      <>
        Point <code>shares add</code> at a directory and your files show up in searches;{' '}
        <code>serve</code> stays online so peers can browse and download them.
      </>
    ),
  },
  {
    glyph: '◈',
    color: 'var(--accent-text)',
    title: 'Daemon mode',
    cmd: 'soulseek-rs daemon',
    body: (
      <>
        Hold one session and let every other command borrow it. The network allows one
        login per account, so this is the neighbourly way to run several at once. It
        speaks JSON-RPC, so other tools can drive it too.
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
    cmd: 'soulseek-rs room list',
    body: <>List, join, and talk in public rooms, several open at once as tabs.</>,
  },
  {
    glyph: '@',
    color: 'var(--status-info)',
    title: 'Private messages',
    cmd: 'soulseek-rs message send <user> "hi"',
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
    cmd: 'soulseek-rs search "rjd2" --json',
    body: (
      <>
        A full terminal interface, plus scriptable subcommands: stdout is data, stderr is
        progress, the exit code is the verdict.
      </>
    ),
  },
] as const

const Hero = () => (
  <section className="pt-5 pb-8 sm:pt-7 sm:pb-10">
    <Wrap>
      <div className="flex flex-col gap-[22px]">
        <Eyebrow className="max-md:hidden">Soulseek client · terminal · Rust</Eyebrow>
        <div className="flex items-center gap-3.5 sm:gap-6 md:items-start md:gap-11">
          <Logo size={172} className="max-md:!h-32 md:mt-2" />
          <h1 className="text-[29px] leading-[37px] font-bold tracking-[-0.01em] text-balance sm:text-[34px] sm:leading-[42px] md:text-display md:leading-[var(--text-display--line-height)]">
            A soulseek client for the terminal. Built for agents and people who live there.
          </h1>
        </div>
        <p className="text-lg leading-[29px] text-pretty text-secondary max-md:hidden">
          Search the network, share your files, browse someone&rsquo;s collection, join a
          room. It runs over ssh on the machine where your music already lives.
        </p>
        <div className="flex flex-wrap items-center justify-between gap-x-5 gap-y-3 sm:gap-y-4">
          <div className="flex flex-wrap items-center gap-2 max-md:w-full max-md:justify-center sm:gap-3">
            <Button href="/install" variant="accent">
              Install
            </Button>
            <Button href="/docs">Read the docs</Button>
            <Button href={LINKS.gh}>GitHub</Button>
          </div>
          <CommandPill text="brew install michel/tap/soulseek-rs" />
        </div>
      </div>

      <div className="mt-3.5 overflow-hidden">
        <HeroTerminal />
      </div>
    </Wrap>
  </section>
)

const RunsOn = () => (
  <section className="border-t border-hairline py-6">
    <Wrap>
      <div className="flex flex-wrap items-center gap-3.5 sm:gap-8">
        <Eyebrow className="max-[519px]:hidden">Runs on</Eyebrow>
        <ul className="flex w-full flex-nowrap justify-center min-[520px]:flex-wrap min-[520px]:justify-start sm:w-auto">
          {PLATFORMS.map((platform, i) => (
            <li
              key={platform.name}
              className={cn(
                'flex w-auto border-hairline px-[18px] first:border-l-0 first:pl-0 not-first:border-l',
                'min-[520px]:w-full min-[520px]:border-l-0 min-[520px]:px-0',
                i === 0 ? 'sm:w-auto sm:pr-6.5' : 'sm:w-auto sm:border-l sm:px-6.5',
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
)

const About = () => (
  <Section id="about">
    <SectionHead eyebrow="what it is" title="A client for a network that runs on people.">
      No charts, no recommendations. You search, you find the person who has it, you
      download it from them.
    </SectionHead>
    <Cols>
      <Prose>
        <p>
          Soulseek is a peer-to-peer network from the early 2000s. People still use it to
          share niche music by hand: the album that exists in one person&rsquo;s folder and
          nowhere else.
        </p>
        <p>
          soulseek-rs speaks that protocol. It&rsquo;s a third-party client, not the
          official app. It shares, browses, and does rooms and private messages, because a
          client that only downloads takes without giving anything back.
        </p>
      </Prose>
      <div className="flex flex-col gap-4">
        <Callout title="build your own">
          <p>
            Reach for <Code>soulseek-rs-lib</Code> to build a custom client or an agent on
            the protocol.{' '}
            <Link to="/install" className="text-link hover:text-link-hover">
              Use the library
            </Link>
          </p>
        </Callout>
        <Callout title="agents and automation">
          <p>
            Every command runs as a one-off from the shell. Point cron, a script, or an
            agent at them and read JSON records back off stdout. Nothing to stand up first;{' '}
            <Code>soulseek-rs daemon</Code> is one command when a batch of them should share
            a single login, and the agent skill starts one for you.{' '}
            <Code>skills install</Code> teaches the agent the surface.{' '}
            <Link to="/docs" className="text-link hover:text-link-hover">
              See the commands
            </Link>
          </p>
        </Callout>
      </div>
    </Cols>
  </Section>
)

const Features = () => (
  <Section id="features">
    <SectionHead eyebrow="features" title="What it does.">
      Everything happens in the TUI, and every command also runs as a one-off from the
      shell.
    </SectionHead>
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
)

const Agents = () => (
  <Section id="agents">
    <SectionHead eyebrow="agent native" title="Ask for the record. Skip the pipeline.">
      An agent can run any CLI. <Code>skills install</Code> teaches it to run this one well.
    </SectionHead>
    <img
      src={`${import.meta.env.BASE_URL}agent-demo.svg`}
      alt="an agent loading the soulseek-rs skill and downloading tracks"
      className="w-full"
    />
    <Prose className="mt-11 max-w-[660px]">
      <p>
        <Code>--help</Code> lists flags. It cannot say which JSON keys a record carries, or
        that exit 4 means widen the query rather than retry it. An agent without that
        guesses, and guessing against a stranger&rsquo;s file server hangs the job.
      </p>
      <p>
        <Code>skills install</Code> fills that in. The requests stop being pipelines you
        write: mirror a peer&rsquo;s FLAC folder overnight, or check whether a user is worth
        queueing behind before a 2 GB transfer.
      </p>
      <p>
        It stays a CLI: no MCP server, no API key. The skill tells your agent to start a
        daemon first, so a batch of parallel requests is one login rather than one per
        command. Upgrading the binary upgrades the skill; rerun <Code>skills install</Code>{' '}
        to take it.{' '}
        <Link to="/docs" className="text-link hover:text-link-hover">
          See the commands
        </Link>
      </p>
    </Prose>
  </Section>
)

const GetStarted = () => (
  <Section band>
    <Cols center>
      <div className="flex flex-col gap-3.5">
        <Eyebrow>get started</Eyebrow>
        <h2 className="text-[26px] leading-[34px] font-medium text-balance sm:text-title sm:leading-[var(--text-title--line-height)]">
          Install it, then press <span className="text-accent-text">s</span> to search.
        </h2>
        <p className="text-secondary">
          One command on macOS or Linux, then the TUI opens. No Rust toolchain needed.
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
          { t: 'cmd', text: 'brew install michel/tap/soulseek-rs' },
          { t: 'cmd', text: 'soulseek-rs' },
        ]}
      />
    </Cols>
  </Section>
)

export const Home = () => (
  <>
    <Hero />
    <RunsOn />
    <About />
    <Features />
    <Agents />
    <GetStarted />
  </>
)
