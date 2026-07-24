import { Button } from '@/components/ui/button'
import { Eyebrow, Wrap } from '@/components/ui/layout'
import { Terminal } from '@/components/ui/terminal'

export const NotFound = () => {
  return (
    <section className="py-16">
      <Wrap>
        <div className="flex max-w-[560px] flex-col gap-3.5">
          <Eyebrow>404</Eyebrow>
          <h1 className="text-[26px] leading-[34px] font-medium sm:text-title sm:leading-[var(--text-title--line-height)]">
            No such path.
          </h1>
          <p className="text-secondary">
            That page doesn&rsquo;t exist. The four that do are in the nav above.
          </p>
          <Terminal
            lines={[
              { t: 'cmd', text: 'soulseek-rs --help' },
              { t: 'cm', text: '# no such page: check the nav, or start here' },
            ]}
            copy="soulseek-rs --help"
          />
          <div className="mt-1.5 flex flex-wrap gap-2 sm:gap-3">
            <Button href="/" variant="accent">
              Home
            </Button>
            <Button href="/docs">Read the docs</Button>
          </div>
        </div>
      </Wrap>
    </section>
  )
}
