import { Link } from 'react-router'

import { Button } from '@/components/ui/button'
import { Code } from '@/components/ui/inline-code'
import {
  Cols,
  Eyebrow,
  PageHead,
  Prose,
  Section,
  SectionHead,
} from '@/components/ui/layout'
import { Callout, Panel } from '@/components/ui/panel'
import { Terminal } from '@/components/ui/terminal'
import { LINKS } from '@/lib/links'

const GLOBAL_KEYS: readonly (readonly [string, string])[] = [
  ['Space', 'select a result'],
  ['Enter', 'download / send'],
  ['b', "browse the owner's files"],
  ['c', 'chat rooms'],
  ['m', 'compose a private message'],
  ['i', 'inbox (unread counter)'],
  ['/', 'filter the current list'],
  ['a / A', 'select all / none'],
  ['1–3', 'focus a pane'],
  ['q', 'quit'],
]

const ROOM_KEYS: readonly (readonly [string, string])[] = [
  ['Enter', 'join room / send message'],
  ['Tab · ⇧Tab', 'switch between open rooms'],
  ['x', 'leave the active room'],
  ['l', 'back to the room list'],
  ['↑ ↓', 'select a member'],
  ['b', "browse the member's files"],
  ['m', 'message the member'],
]

const PANES: readonly { num?: number; title: string; body: string }[] = [
  {
    num: 1,
    title: 'Searches',
    body: 'Your queries and how many results each returned. Type / to search the network; each search keeps its own result set.',
  },
  {
    num: 2,
    title: 'Results',
    body: 'Files from the network: size, user, bitrate, speed, free slots. Space selects, Enter queues a download.',
  },
  {
    num: 3,
    title: 'Downloads / Uploads',
    body: 'Live transfers in both directions with progress and speed: queued, active, complete, failed.',
  },
  {
    title: 'Info',
    body: 'Everything about the selected file: user, size, path, bitrate, length, queue position, free slots.',
  },
]

const KeyRow = ({ combo, description }: { combo: string; description: string }) => (
  <div className="grid grid-cols-[120px_1fr] items-baseline gap-3 border-b border-hairline py-[7px]">
    <span>
      <span className="inline-block min-w-5 rounded-md border border-line px-[7px] text-center text-xs text-primary shadow-[0_1px_0_0_var(--border-default)]">
        {combo}
      </span>
    </span>
    <span className="text-[13px] text-secondary">{description}</span>
  </div>
)

export const Docs = () => {
  return (
    <>
      <PageHead
        eyebrow="docs · quick start"
        title="Install it, run it, drive it from the keyboard."
      >
        This is the short version. The full reference (every flag, every message) lives in
        the README and on docs.rs.
      </PageHead>

      <Section>
        <SectionHead eyebrow="1 · first run" title="Two commands to a running client.">
          You need a Rust toolchain. After <Code>cargo install</Code>, the binary is on your
          PATH.
        </SectionHead>
        <Cols start>
          <Terminal
            lines={[
              { t: 'cmd', text: 'cargo install soulseek-rs' },
              { t: 'cmd', text: 'soulseek-rs' },
              { t: 'cm', text: '# the TUI opens; log in or register on first run' },
            ]}
          />
          <Prose>
            <p>
              On first run soulseek-rs shows a login screen: sign in with an existing
              Soulseek account or register a new one, right in the TUI. Your password is
              stored in the OS keychain, not in plain text.
            </p>
            <p>
              Downloads land in a conventional folder by default, and your TUI state
              (searches, downloads, rooms) is saved and restored across restarts.
            </p>
          </Prose>
        </Cols>
      </Section>

      <Section>
        <SectionHead eyebrow="2 · the layout" title="Four panes.">
          The whole client is these four boxes. Press <Code>1</Code>–<Code>3</Code> to focus
          one; the focused pane&rsquo;s legend turns green.
        </SectionHead>
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          {PANES.map((pane) => (
            <Panel key={pane.title} num={pane.num} title={pane.title}>
              <p className="text-[13px] leading-[22px] text-secondary">{pane.body}</p>
            </Panel>
          ))}
        </div>
      </Section>

      <Section>
        <SectionHead eyebrow="3 · keys" title="It's all keyboard.">
          The bottom bar always shows the keys for where you are. The essentials:
        </SectionHead>
        <Cols>
          <div>
            <Eyebrow className="mb-4 block">anywhere</Eyebrow>
            <div className="flex flex-col">
              {GLOBAL_KEYS.map(([combo, description]) => (
                <KeyRow key={combo} combo={combo} description={description} />
              ))}
            </div>
          </div>
          <div>
            <Eyebrow className="mb-4 block">
              in the chat-rooms popup <span className="text-muted">(press c)</span>
            </Eyebrow>
            <div className="flex flex-col">
              {ROOM_KEYS.map(([combo, description]) => (
                <KeyRow key={combo} combo={combo} description={description} />
              ))}
            </div>
          </div>
        </Cols>
      </Section>

      <Section>
        <SectionHead
          eyebrow="4 · from the shell"
          title="Every command also runs as a one-off."
        >
          Skip the TUI when you&rsquo;re scripting or mid-task.
        </SectionHead>
        <Cols start>
          <Terminal
            label="shell"
            lines={[
              { t: 'cmd', text: 'soulseek-rs "public domain field recordings"' },
              { t: 'cm', text: '# open the TUI straight onto a search' },
              { t: 'cmd', text: 'soulseek-rs message <username> "hello there"' },
              { t: 'cmd', text: 'soulseek-rs rooms' },
              { t: 'cm', text: '# list public rooms, busiest first' },
              { t: 'cmd', text: 'soulseek-rs chat <room> "hello room"' },
              { t: 'cmd', text: 'soulseek-rs portmap' },
            ]}
          />
          <Prose>
            <p>
              The subcommands mirror the TUI: <Code>search</Code>, <Code>message</Code>,{' '}
              <Code>browse</Code>, <Code>rooms</Code>, <Code>chat</Code>, and{' '}
              <Code>portmap</Code>. Pass a room and a message to <Code>chat</Code> to say
              one thing and exit.
            </p>
            <p>
              <Code>portmap</Code> tests whether your router will let peers connect back,
              worth running once.{' '}
              <Link to="/install" className="text-link hover:text-link-hover">
                More on being reachable
              </Link>
            </p>
          </Prose>
        </Cols>
        <div className="mt-6">
          <Callout tone="warn" title="the one limit worth knowing up front">
            <p>
              If both you and a peer are behind routers with no forwarded port, browsing
              that peer can&rsquo;t work, that&rsquo;s a fundamental Soulseek/peer-to-peer
              limitation, not a bug.
            </p>
          </Callout>
        </div>
      </Section>

      <Section band>
        <Panel title="the rest" className="bg-raised [&>span]:bg-panel">
          <Cols center className="gap-6">
            <Prose>
              <p>
                Deeper docs live where they stay current: the full README on GitHub, and the
                generated API reference for the library on docs.rs.
              </p>
            </Prose>
            <div className="flex flex-wrap gap-2 sm:gap-3">
              <Button href={LINKS.gh} variant="accent">
                README
              </Button>
              <Button href={LINKS.docsrs}>docs.rs</Button>
              <Button href={LINKS.changelog}>Changelog</Button>
            </div>
          </Cols>
        </Panel>
      </Section>
    </>
  )
}
