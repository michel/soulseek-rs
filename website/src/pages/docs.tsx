import { Link } from 'react-router'

import { HeroTerminal } from '@/components/tui/hero-terminal'
import { Button } from '@/components/ui/button'
import { ExtLink } from '@/components/ui/ext-link'
import { Code } from '@/components/ui/inline-code'
import {
  Cols,
  Eyebrow,
  Prose,
  Section,
  SectionHead,
} from '@/components/ui/layout'
import { Callout, Panel } from '@/components/ui/panel'
import { Terminal } from '@/components/ui/terminal'
import { LINKS } from '@/lib/links'

const GLOBAL_KEYS: readonly (readonly [string, string])[] = [
  ['s', 'search the network'],
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
    body: 'Your queries and how many results each returned. Press s to search the network; each search keeps its own result set.',
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

const FirstRun = () => (
  <Section flush className="pt-10">
    <SectionHead eyebrow="1 · first run" title="One command to a running client.">
      With <Code>soulseek-rs</Code> on your PATH, running it opens the TUI.{' '}
      <Link to="/install" className="text-link hover:text-link-hover">
        Installing it
      </Link>{' '}
      is one command on macOS, Linux and Windows.
    </SectionHead>
    <Cols start>
      <Terminal
        lines={[
          { t: 'cmd', text: 'soulseek-rs' },
          { t: 'cm', text: '# the TUI opens; log in or register on first run' },
        ]}
      />
      <Prose>
        <p>
          On first run you get a login screen: sign in with an existing Soulseek account or
          register a new one. If a daemon is already running, the TUI attaches to it and
          asks for nothing. Your password goes to the OS keychain, not a plain-text file.
        </p>
        <p>
          Downloads land in a conventional folder, and TUI state (searches, downloads,
          rooms) is restored across restarts.
        </p>
      </Prose>
    </Cols>
  </Section>
)

const Layout = () => (
  <Section>
    <SectionHead eyebrow="2 · the layout" title="Four panes.">
      The whole client is these four boxes. Press <Code>1</Code>–<Code>3</Code> to focus
      one; the focused pane&rsquo;s legend turns green.
    </SectionHead>
    <Cols start>
      <div className="overflow-hidden">
        <HeroTerminal />
      </div>
      <div className="grid grid-cols-1 gap-4">
        {PANES.map((pane) => (
          <Panel key={pane.title} num={pane.num} title={pane.title}>
            <p className="text-[13px] leading-[22px] text-secondary">{pane.body}</p>
          </Panel>
        ))}
      </div>
    </Cols>
  </Section>
)

const Keys = () => (
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
)

const FromTheShell = () => (
  <Section>
    <SectionHead eyebrow="4 · from the shell" title="Every command also runs as a one-off.">
      Skip the TUI when you&rsquo;re scripting or mid-task. stdout is data, stderr is
      progress, and the exit code is the verdict.
    </SectionHead>
    <Cols start>
      <Terminal
        label="shell"
        lines={[
          { t: 'cmd', text: 'soulseek-rs get "public domain field recordings"' },
          { t: 'cm', text: '# search, pick the best, download it' },
          { t: 'cmd', text: 'soulseek-rs search "netlabel 2004" --min-bitrate 320' },
          { t: 'cmd', text: 'soulseek-rs browse <username> --json' },
          { t: 'cmd', text: 'soulseek-rs wish add "rare pressing"' },
          { t: 'cm', text: '# a standing search, re-run by serve --wishlist' },
          { t: 'cmd', text: 'soulseek-rs room list' },
          { t: 'cm', text: '# public rooms, busiest first' },
          { t: 'cmd', text: 'soulseek-rs room say <room> "hello room"' },
          { t: 'cmd', text: 'soulseek-rs message send <username> "hello there"' },
          { t: 'cmd', text: 'soulseek-rs serve --follow' },
          { t: 'cm', text: '# stay online sharing; every upload as it happens' },
          { t: 'cmd', text: 'soulseek-rs skills install' },
          { t: 'cm', text: '# hand all of the above to your coding agent' },
        ]}
      />
      <Prose>
        <p>
          The command groups: <Code>search</Code>, <Code>get</Code>, <Code>download</Code>,{' '}
          <Code>browse</Code>, <Code>wish</Code>, <Code>room</Code>, <Code>message</Code>,{' '}
          <Code>serve</Code>, <Code>user</Code>, <Code>whoami</Code>, <Code>shares</Code>,{' '}
          <Code>daemon</Code>, <Code>config</Code>, <Code>portmap</Code>,{' '}
          <Code>skills</Code>, <Code>completions</Code>. Records are tab-separated fields,
          one per line, or NDJSON with <Code>--json</Code>.
        </p>
        <p>
          No TTY required. <Code>search --json</Code> pipes through <Code>jq</Code> into{' '}
          <Code>download --stdin</Code>. Every ending has its own exit code: 4 found
          nothing, 3 could not reach the server, 6 the transfer died, 7 another login took
          the session away, the one a daemon spares you.
        </p>
        <p>
          <Code>completions install</Code> gives bash, zsh and fish tab completion for all
          of it, generated from the same definition the binary parses with.{' '}
          <Code>completions uninstall</Code> takes it back out.
        </p>
        <p>
          <Code>portmap</Code> tests whether your router will let peers connect back, worth
          running once.{' '}
          <Link to="/install" className="text-link hover:text-link-hover">
            More on being reachable
          </Link>
        </p>
      </Prose>
    </Cols>
    <div className="mt-6">
      <Callout tone="warn" title="the one limit worth knowing up front">
        <p>
          If both you and a peer are behind routers with no forwarded port, browsing that
          peer can&rsquo;t work, that&rsquo;s a fundamental Soulseek/peer-to-peer
          limitation, not a bug.
        </p>
      </Callout>
    </div>
  </Section>
)

const SubHead = ({
  eyebrow,
  title,
  children,
}: {
  eyebrow: string
  title: string
  children: React.ReactNode
}) => (
  <div className="mt-13 mb-9 flex max-w-[660px] flex-col gap-3">
    <Eyebrow>{eyebrow}</Eyebrow>
    <h3 className="text-[20px] leading-[28px] font-medium tracking-[-0.005em] text-balance">
      {title}
    </h3>
    <p className="text-base leading-[26px] text-pretty text-secondary">{children}</p>
  </div>
)

const Daemon = () => (
  <Section>
    <SectionHead eyebrow="5 · the daemon" title="One login, however many commands.">
      Soulseek allows one session per account. Run a daemon and everything else borrows
      it. No second login, nothing to configure.
    </SectionHead>
    <Cols start>
      <Terminal
        label="shell"
        lines={[
          { t: 'cmd', text: 'soulseek-rs daemon &' },
          { t: 'cm', text: '# once; it holds the session' },
          { t: 'cmd', text: 'soulseek-rs search "netlabel 2004" --json' },
          { t: 'cm', text: '# no login, no flags; it finds the daemon' },
          { t: 'cmd', text: 'soulseek-rs search "netlabel 2004" --no-daemon' },
          { t: 'cm', text: "# opt out; open this run's own session" },
          { t: 'cmd', text: 'soulseek-rs' },
          { t: 'cm', text: '# the TUI attaches to the same session' },
          { t: 'cmd', text: 'soulseek-rs daemon status' },
          { t: 'cmd', text: 'soulseek-rs daemon stop' },
        ]}
      />
      <Prose>
        <p>
          Commands find it themselves: it listens on a Unix socket only your user can open,
          so there is no address to pass and no token to handle. Without one running,
          everything works as it did before, and <Code>--no-daemon</Code> gets that back for
          a single command. <Code>daemon status</Code> exits 3 when none is running, which is
          how a script decides whether to start one. Windows has no Unix sockets: start the
          daemon with <Code>--bind 127.0.0.1:5030</Code> and point commands at it with{' '}
          <Code>config set daemon</Code> and <Code>config set daemon_token</Code>.
        </p>
        <p>
          Every window is a view of the same session. Open a second TUI and it shows the
          searches and transfers already running; queue something in either and both see it.
          No login screen, because the daemon is already signed in, and the inbox opens on
          the private messages it collected while nobody was watching. The settings form
          edits the daemon&rsquo;s download folder and share list, so paths you type there
          name its filesystem, not this machine&rsquo;s. Close a window and its downloads
          keep going: they belong to the daemon.
        </p>
        <p>
          Attach one and commands report its world: <Code>whoami</Code> names its account
          and server, <Code>download</Code> prints the path on its disk, and{' '}
          <Code>serve --follow</Code> watches its uploads instead of starting a second
          server. It holds the session, not the wishlist: standing searches re-run under{' '}
          <Code>serve --wishlist</Code>, or when you call <Code>wish run</Code>.
        </p>
        <p>
          Your username is your standing on Soulseek, and the server permits one session per
          account, so two one-off commands at once cut each other off. A daemon is one login
          under that name for as long as you work, shares indexed once and reachable the
          whole time instead of going dark between invocations, and downloads that outlive
          the command that queued them. If you drive this from a script or an agent, start
          one.
        </p>
      </Prose>
    </Cols>

    <SubHead eyebrow="use case · an always-on box" title="Put it on a machine you never turn off.">
      A home server, a NAS, a Raspberry Pi. No terminal, nobody logged in. Downloads keep
      going after you close the lid, and your shares stay reachable instead of vanishing
      when the laptop sleeps.
    </SubHead>
    <Cols start>
      <Terminal
        label="on the box"
        lines={[
          { t: 'cmd', text: 'soulseek-rs shares add /srv/music' },
          { t: 'cm', text: '# what the box offers the network' },
          { t: 'cmd', text: 'soulseek-rs daemon --bind 0.0.0.0:5030' },
          { t: 'cmd', text: 'soulseek-rs daemon token' },
          { t: 'cm', text: '# copy this once' },
          { t: 'cm', text: '' },
          { t: 'cm', text: '# on your laptop, one-time setup:' },
          { t: 'cmd', text: 'soulseek-rs config set daemon nas.local:5030' },
          { t: 'cmd', text: 'soulseek-rs config set daemon_token <token>' },
          { t: 'cm', text: '' },
          { t: 'cm', text: '# then just use it, no flags:' },
          { t: 'cmd', text: 'soulseek-rs get "selected ambient works"' },
          { t: 'cmd', text: 'soulseek-rs daemon status' },
          { t: 'cmd', text: 'soulseek-rs daemon stop' },
          { t: 'cm', text: '# shuts down the remote one when you are done' },
        ]}
      />
      <Prose>
        <p>
          Your laptop needs no Soulseek account of its own; it uses the box&rsquo;s. Queue
          what you want and close the lid: the downloads belong to the daemon, not the
          command that asked for them. The box answers searches and serves uploads the whole
          time, which a sleeping laptop cannot.
        </p>
        <p>
          That port has no encryption. Fine on a home network. Over the internet, reach it
          through SSH instead: <Code>ssh -L 5030:localhost:5030 nas</Code>, or run the
          commands on the box.
        </p>
      </Prose>
    </Cols>
    <div className="mt-6">
      <Callout tone="warn" title="pushed settings are live only">
        <p>
          <Code>config set download_dir</Code> and <Code>shares add</Code> reach the running
          daemon and take effect at once, but the box reads its own <Code>config.toml</Code>{' '}
          when it restarts, so make lasting changes there. <Code>shares add</Code> also
          checks the folder on the machine you type it on, so a path that exists only on the
          box fails from your laptop: add shares on the box, or in the attached TUI&rsquo;s
          settings form, which sends paths through as typed.
        </p>
      </Callout>
    </div>

    <SubHead eyebrow="build your own" title="The interface is open.">
      Nothing about the daemon is specific to this CLI. Point a client generator at its
      schema and write your own remote.
    </SubHead>
    <Cols start>
      <Prose>
        <p>
          It speaks newline-delimited JSON-RPC 2.0, described by an{' '}
          <ExtLink href="https://open-rpc.org">OpenRPC</ExtLink> document. The daemon serves
          that document itself through <Code>rpc.discover</Code>, so a client can ask it what
          it can do. The same document is checked in at{' '}
          <ExtLink href={`${LINKS.gh}/blob/master/docs/openrpc.json`}>
            docs/openrpc.json
          </ExtLink>{' '}
          on GitHub. <Code>auth</Code> answers with the protocol version, 1 today, so a
          client written against an older shape learns it at connect time.
        </p>
        <p>
          OpenRPC is the JSON-RPC counterpart to OpenAPI: point a generator at it and you
          get typed bindings in whatever language you work in. Build the phone app that
          queues downloads on your home server from the sofa, or a web page showing what is
          transferring. Nobody to ask first.
        </p>
        <p>
          Searches, transfers, rooms, private messages and shares all sit on the same
          interface this CLI uses, with no private back channel it keeps for itself. Live
          updates are pushed to every connected client rather than polled for:{' '}
          <Code>event.room</Code>, <Code>event.message</Code>, <Code>event.upload</Code>,{' '}
          <Code>event.download_status</Code>, <Code>event.browse</Code> and{' '}
          <Code>event.session_loss</Code>. One rule to write against: the download folder
          belongs to the daemon, so <Code>download.set_dir</Code> moves it for every
          transfer and a per-download destination is ignored.
        </p>
      </Prose>
      <Terminal
        label="any language"
        lines={[
          { t: 'cmd', text: 'soulseek-rs daemon --bind 0.0.0.0:5030' },
          { t: 'cm', text: '# then, from your own client:' },
          { t: 'out', text: '{"jsonrpc":"2.0","id":1,"method":"auth",' },
          { t: 'out', text: ' "params":{"protocol":1,"token":"..."}}' },
          { t: 'out', text: '{"jsonrpc":"2.0","id":2,"method":"search.start",' },
          { t: 'out', text: ' "params":{"query":"aphex twin"}}' },
          { t: 'cm', text: '# events arrive unasked: transfers, chat, uploads' },
        ]}
      />
    </Cols>
  </Section>
)

const TheRest = () => (
  <Section band>
    <Panel title="the rest" className="bg-raised [&>span]:bg-panel">
      <Cols center className="gap-6">
        <Prose>
          <p>
            The README on GitHub and the library API reference on docs.rs stay current with
            the code.
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
)

export const Docs = () => (
  <>
    <FirstRun />
    <Layout />
    <Keys />
    <FromTheShell />
    <Daemon />
    <TheRest />
  </>
)
