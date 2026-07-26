import { MacWindow, ShortcutBar, StatusLine } from './chrome'
import { SHARE } from './data'
import { InfoPane, ResultsPane, SearchesPane, TransfersPane } from './panes'
import { useFitScale } from './use-fit-scale'
import { useHeroDemo } from './use-hero-demo'

const DESIGN_WIDTH = 1360
const DESIGN_HEIGHT = 863

export const HeroTerminal = () => {
  const demo = useHeroDemo()
  const { wrapRef, innerRef, scale, height, ready } = useFitScale(DESIGN_WIDTH)

  return (
    <div
      ref={wrapRef}
      className="w-full overflow-hidden"
      style={ready ? { height } : { aspectRatio: `${String(DESIGN_WIDTH)} / ${String(DESIGN_HEIGHT)}` }}
    >
      <div style={ready ? { width: DESIGN_WIDTH * scale, height } : undefined}>
        <div
          ref={innerRef}
          className="origin-top-left"
          style={{
            width: DESIGN_WIDTH,
            transform: ready ? `scale(${String(scale)})` : undefined,
            visibility: ready ? 'visible' : 'hidden',
          }}
        >
          <MacWindow>
            <div
              tabIndex={0}
              role="application"
              aria-label="Interactive soulseek-rs terminal demo"
              onKeyDown={demo.onKeyDown}
              className="tui-content p-2.5 outline-none"
            >
              <StatusLine counts={demo.counts} pct={demo.pct} />

              <div className="mt-3 grid gap-2.5" style={{ gridTemplateColumns: '300px 1fr' }}>
                <div className="min-h-[498px]">
                  <SearchesPane
                    searches={demo.searches}
                    activeId={demo.activeId}
                    active={demo.focus === 1}
                    onPick={demo.pickSearch}
                    onSubmit={demo.submitSearch}
                  />
                </div>

                <div className="grid min-h-0 gap-2.5" style={{ gridTemplateRows: '1fr 300px' }}>
                  <div className="relative min-h-[250px] overflow-hidden">
                    <ResultsPane
                      query={demo.query}
                      rows={demo.rows}
                      selected={demo.selected}
                      active={demo.focus === 2}
                      onSelect={demo.selectRow}
                      onQueue={demo.queueRow}
                    />
                    {demo.searching && (
                      <div className="pointer-events-none absolute inset-0 flex items-center justify-center bg-[color-mix(in_srgb,var(--color-sleeve)_35%,transparent)] text-[13.5px] tracking-[0.02em] text-[var(--color-dust)]">
                        searching the network
                        <span className="ml-1.5 inline-block h-[15px] w-2 translate-y-[2px] rounded-[1.5px] bg-[var(--color-oxide)] motion-safe:animate-cursor-blink" />
                      </div>
                    )}
                  </div>

                  <div
                    className="grid min-h-0 gap-2.5"
                    style={{ gridTemplateColumns: '1fr 340px' }}
                  >
                    <div className="min-h-0 min-w-0 overflow-hidden">
                      <TransfersPane rows={demo.transfers.slice(0, 7)} active={demo.focus === 3} />
                    </div>
                    <div className="min-h-0 min-w-0 overflow-hidden">
                      <InfoPane row={demo.infoRow} />
                    </div>
                  </div>
                </div>
              </div>

              <ShortcutBar share={SHARE} />
            </div>
          </MacWindow>
        </div>
      </div>
    </div>
  )
}
