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
        Every command runs one-shot and exits: JSON on stdout, an exit code, nothing left
        running. No resident queue to pause, resume, or drive remotely. slskd has that.
      </>
    ),
    links: [{ label: 'slskd', href: LINKS.slskd }],
  },
]

const CITIZENSHIP = [
  {
    glyph: '↑',
    color: 'var(--status-success)',
    title: 'Share, and share real files',
    body: (
      <>
        Point <code>shares add</code> at the collection you listen to. Full albums,
        honest bitrates, nothing transcoded up from a lossy source and relabelled.
      </>
    ),
  },
  {
    glyph: '≡',
    color: 'var(--status-info)',
    title: 'Organize so others can find it',
    body: (
      <>
        Artist and album folders, tagged files, readable names. Someone else&rsquo;s search
        only reaches you if your filenames say what the music is.
      </>
    ),
  },
  {
    glyph: '∞',
    color: 'var(--accent-text)',
    title: 'Stay online',
    body: (
      <>
        Leave <code>serve --follow</code> running in a tmux session on the box that is
        already on. A share that appears for an hour a week is close to no share at all.
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
    title: 'Answer people',
    body: (
      <>
        Rooms and private messages are part of the client for a reason. Someone asking
        where the rest of a discography went is worth a reply.
      </>
    ),
  },
  {
    glyph: '⋯',
    color: 'var(--text-secondary)',
    title: 'Wait your turn',
    body: (
      <>
        Queues are one person&rsquo;s upload bandwidth being divided up. Queue it, leave it,
        come back later. Do not requeue the same file to jump the line.
      </>
    ),
  },
] as const

export const Community = () => {
  return (
    <>
      <PageHead
        eyebrow="community"
        title="Help out, or go where you’re better served."
      >
        It&rsquo;s mostly one person who&rsquo;s been on Soulseek since the early 2000s.
        Issues and pull requests are welcome, and if another client fits you better, use it.
      </PageHead>

      <Section>
        <SectionHead eyebrow="contributing" title="How to help.">
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
              suite against{' '}
              <ExtLink href={LINKS.soulfind}>soulfind</ExtLink>{' '}
              run in CI, so a green checkmark means it built and talked to a server.
            </p>
            <p>
              soulfind is an open-source Soulseek server. Run it locally and you develop
              against a real protocol implementation without touching the public network.
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
          eyebrow="being a good member"
          title="The network is other people's hard drives."
        >
          Decades of records, live sets, and pressings that never made it anywhere else,
          kept online by people who care about them. None of this is enforced. It is just
          how the network stays worth using.
        </SectionHead>
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
      </Section>

      <Section>
        <SectionHead
          eyebrow="what this is not"
          title="Three good clients to point you elsewhere."
        >
          soulseek-rs has no GUI and no daemon mode yet. If one of these fits you better,
          use it.
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
              Free to use, copy, modify, and distribute. Copyright © 2026 Michel de Graaf.
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
