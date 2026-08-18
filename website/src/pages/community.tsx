import { Button } from '@/components/ui/button'
import { ExtLink } from '@/components/ui/ext-link'
import { FeatureCard } from '@/components/ui/feature-card'
import { Code } from '@/components/ui/inline-code'
import { Cols, Eyebrow, PageHead, Prose, Section, SectionHead } from '@/components/ui/layout'
import { Callout } from '@/components/ui/panel'
import { Terminal } from '@/components/ui/terminal'
import { LINKS } from '@/lib/links'

interface Alternative {
  question: string
  answer: React.ReactNode
  links: readonly { label: string; href: string }[]
}

const ALTERNATIVES: readonly Alternative[] = [
  {
    question: 'Want a window and a mouse?',
    answer: <>SoulseekQt or Nicotine+. Both are good.</>,
    links: [
      { label: 'SoulseekQt', href: LINKS.slsknet },
      { label: 'Nicotine+', href: LINKS.nicotine },
    ],
  },
  {
    question: 'On a Mac and want it to feel like one?',
    answer: (
      <>
        SeeleSeek is native Swift and SwiftUI rather than Qt or Python, so it opens like
        any other Mac app. macOS 15 or later, MIT licensed.
      </>
    ),
    links: [{ label: 'SeeleSeek', href: LINKS.seeleseek }],
  },
  {
    question: 'Want feature completeness?',
    answer: (
      <>
        Nicotine+ has more: wishlists, user lists, transfer rules, plugins, translations.
        It&rsquo;s mature and well run.
      </>
    ),
    links: [{ label: 'Nicotine+', href: LINKS.nicotine }],
  },
  {
    question: 'Want a queue that outlives the command?',
    answer: (
      <>
        <Code>soulseek-rs daemon</Code> holds one: pause, resume and remove act on it from
        the TUI or over JSON-RPC, and <Code>--bind</Code> lets another machine drive it.
        slskd still has the web interface and accounts for several people.
      </>
    ),
    links: [{ label: 'slskd', href: LINKS.slskd }],
  },
]

const CITIZENSHIP = [
  {
    glyph: '↑',
    color: 'var(--status-success)',
    title: 'Share something, anything',
    body: (
      <>
        Point <code>shares add</code> at the collection you listen to. Full albums,
        honest bitrates, nothing transcoded up from a lossy source and relabelled. Nobody
        checks whether you reshared the exact file you took, only whether you offer
        anything at all.
      </>
    ),
  },
  {
    glyph: '≡',
    color: 'var(--status-info)',
    title: 'Name it so a search can find it',
    body: (
      <>
        Artist, album, year, track. Most people search rather than browse, so your
        filenames are the only route to you. One flat folder of untagged rips is
        invisible.
      </>
    ),
  },
  {
    glyph: '∞',
    color: 'var(--accent-text)',
    title: 'Stay online',
    body: (
      <>
        Leave <code>serve --follow</code> running in tmux on the box that is already on,
        or hand <code>daemon</code> to systemd. A share that appears an hour a week barely
        counts.
      </>
    ),
  },
  {
    glyph: '↔',
    color: 'var(--status-warning)',
    title: 'Let people reach you',
    body: (
      <>
        Forward your listen port or let <code>portmap</code> try UPnP, and leave a free
        upload slot. Two firewalled peers cannot trade with each other.
      </>
    ),
  },
  {
    glyph: '@',
    color: 'var(--status-info)',
    title: 'Ask before you take a discography',
    body: (
      <>
        <code>user &lt;name&gt;</code> shows how someone is set up, and many spell out
        their limits in their profile. One line from <code>message send</code> beats
        getting banned mid-queue. Answer the people who write to you, too.
      </>
    ),
  },
  {
    glyph: '⇅',
    color: 'var(--status-success)',
    title: 'Size your slots for your uplink',
    body: (
      <>
        <code>--upload-slots</code> is not a score. Set it past what your uplink can feed
        and everyone in the queue gets a trickle. Start low, raise it when uploads finish
        fast.
      </>
    ),
  },
  {
    glyph: '⋯',
    color: 'var(--text-secondary)',
    title: 'Wait your turn',
    body: (
      <>
        A queue is one person&rsquo;s upload bandwidth divided up. Queue it, leave it,
        come back later. Requeuing to jump the line burns a slot someone else was waiting
        on.
      </>
    ),
  },
  {
    glyph: '⚑',
    color: 'var(--status-warning)',
    title: 'Keep the files reachable',
    body: (
      <>
        Sharing off an external drive works until it is unmounted. Run{' '}
        <code>shares reindex</code> once it is back, or your listing promises files that
        error out when somebody asks.
      </>
    ),
  },
] as const

export const Community = () => {
  return (
    <>
      <PageHead eyebrow="community" title="Share what you take.">
        Soulseek exists because its users leave their collections open to each other. What
        that asks of you, how to help with the client, and where to go if another one fits
        you better.
      </PageHead>

      <Section>
        <SectionHead
          eyebrow="sharing is caring"
          title="The network is other people's hard drives."
        >
          Every file you pull came off a machine somebody left running and spent years
          filling. Records and live sets that exist nowhere else stay online because those
          people keep them there. Download and share nothing back and you are a leech.
        </SectionHead>
        <Cols start className="mb-9">
          <Prose>
            <p>
              The protocol has no ratio and no tracker. Users decide for themselves who to
              serve, and they check what you share before they queue you.
            </p>
            <p>
              Sharing costs you a directory and a process that stays up. It buys you a
              catalogue that still exists in ten years, and peers who move you up the
              queue because they browsed your files first.
            </p>
          </Prose>
          <Terminal
            label="stop being a leech"
            lines={[
              { t: 'cmd', text: 'soulseek-rs shares add ~/Music' },
              { t: 'cmd', text: 'soulseek-rs shares status' },
              { t: 'cm', text: '# folders and files the network will see' },
              { t: 'cmd', text: 'soulseek-rs serve --follow' },
              { t: 'cm', text: '# stay online; every upload as it happens' },
            ]}
          />
        </Cols>
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          {CITIZENSHIP.map((item) => (
            <FeatureCard
              key={item.title}
              glyph={item.glyph}
              color={item.color}
              title={item.title}
            >
              {item.body}
            </FeatureCard>
          ))}
        </div>
        <div className="mt-4">
          <Callout tone="warn" title="locked shares, traders and sellers">
            <p>
              Some users restrict their files to buddies, often for fair reasons. A few sit
              on rare releases and ask you to trade or pay for access. Read their profile,
              then walk away from the ones charging admission; they thin out when people
              stop playing along.
            </p>
          </Callout>
        </div>
      </Section>

      <Section>
        <SectionHead eyebrow="contributing" title="How to help with the client.">
          It&rsquo;s mostly one person who&rsquo;s been on Soulseek since the early 2000s.
          File an issue, open a pull request, or just say what&rsquo;s missing. No
          contributor agreement, no template gymnastics.
        </SectionHead>
        <Cols start>
          <Prose>
            <p>
              The project is a Cargo workspace: <Code>soulseek-rs-lib</Code> (the protocol)
              and <Code>soulseek-rs</Code> (the client). The library stays lean on
              dependencies; the client takes them freely.
            </p>
            <p>
              Format and Clippy run clean on every push, and unit tests plus an end-to-end
              suite against <ExtLink href={LINKS.soulfind}>soulfind</ExtLink> run in CI, so a
              green checkmark means it built and talked to a server. soulfind is an
              open-source Soulseek server; run it locally to develop against a real protocol
              implementation without touching the public network.
            </p>
            <div className="mt-1 flex flex-wrap gap-2 sm:gap-3">
              <Button href={LINKS.issues} variant="accent">
                Open an issue
              </Button>
              <Button href={LINKS.gh}>GitHub</Button>
            </div>
          </Prose>
          <Terminal
            label="develop"
            lines={[
              { t: 'cmd', text: 'RUST_LOG=trace cargo run' },
              { t: 'cm', text: '# run with debug + trace output' },
              { t: 'cmd', text: 'cargo test' },
              { t: 'cmd', text: 'cargo clippy' },
              { t: 'cmd', text: 'cargo fmt' },
            ]}
          />
        </Cols>
      </Section>

      <Section>
        <SectionHead
          eyebrow="built on the library"
          title="Projects using soulseek-rs-lib."
        >
          The protocol crate is there for anyone building their own client or
          automation on Soulseek. These projects do.
        </SectionHead>
        <Cols start>
          <Prose>
            <p>
              <ExtLink href={LINKS.seakarr}>seakarr</ExtLink> — automated Soulseek music
              downloader with library quality upgrading: it scans your collection, finds
              albums below your bitrate bar, and fetches better copies.
            </p>
            <p>
              Built something on <Code>soulseek-rs-lib</Code>? Open a pull request to get
              listed, and{' '}
              <ExtLink href={LINKS.minorVersions}>reserve a client minor version</ExtLink>{' '}
              so your project is identifiable on the network.
            </p>
          </Prose>
        </Cols>
      </Section>

      <Section>
        <SectionHead
          eyebrow="what this is not"
          title="Four good clients to point you elsewhere."
        >
          soulseek-rs has no graphical interface. If one of these fits you better, use it.
        </SectionHead>
        <div className="grid grid-cols-1 gap-3.5">
          {ALTERNATIVES.map((alt) => (
            <div
              key={alt.question}
              className="grid grid-cols-1 items-center gap-4 rounded-md border border-hairline bg-panel p-[18px] sm:grid-cols-[1fr_auto] sm:gap-x-6 sm:p-[22px] lg:grid-cols-1"
            >
              <div>
                <div className="mb-1.5 text-[15px] font-medium text-primary">
                  {alt.question}
                </div>
                <div className="text-[13.5px] leading-[22px] text-secondary">
                  {alt.answer}
                </div>
              </div>
              <div className="flex flex-wrap gap-2 sm:gap-3">
                {alt.links.map((link) => (
                  <Button key={link.label} href={link.href}>
                    {link.label}
                  </Button>
                ))}
              </div>
            </div>
          ))}
        </div>
      </Section>

      <Section band>
        <Cols center className="gap-8">
          <div className="flex flex-col gap-3.5">
            <Eyebrow>license &amp; promises</Eyebrow>
            <h2 className="text-[26px] leading-[34px] font-medium sm:text-title sm:leading-[var(--text-title--line-height)]">
              MIT, and no strings.
            </h2>
            <p className="text-secondary">
              Free to use, copy, modify, and distribute. Copyright © 2026 soulseek-rs contributors.
            </p>
            <div className="mt-1 flex flex-wrap gap-2 sm:gap-3">
              <Button href={LINKS.license} variant="accent">
                Read the license
              </Button>
            </div>
          </div>
          <Callout className="bg-raised" title="two things that stay true">
            <p className="mb-1.5 !text-success">No telemetry. Ever.</p>
            <p className="font-forum text-xs !text-muted">
              Not affiliated with or endorsed by the Soulseek project.
            </p>
          </Callout>
        </Cols>
      </Section>
    </>
  )
}
