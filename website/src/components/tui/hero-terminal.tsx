import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'

import { MacWindow, ShortcutBar, StatusLine, type StatusCounts } from './chrome'
import {
  RESULTS,
  SEARCHES,
  SHARE,
  TRANSFERS,
  type ResultRow,
  type SearchRow,
  type TransferRow,
} from './data'
import { InfoPane, ResultsPane, SearchesPane, TransfersPane } from './panes'
import { useFitScale } from './use-fit-scale'

const DESIGN_WIDTH = 1360
const DESIGN_HEIGHT = 863
const TICK_MS = 480
const MAX_CONCURRENT = 2
const RESTING_ROW = 5

type Phase = 'searching' | 'results' | 'interactive'

const isBrowser = typeof window !== 'undefined'

const matches = (query: string): boolean =>
  isBrowser && window.matchMedia(query).matches

const prefersReducedMotion = (): boolean => matches('(prefers-reduced-motion: reduce)')

const sizeMb = (size: string): number => {
  const parsed = Number.parseFloat(size)
  return Number.isNaN(parsed) || parsed === 0 ? 12 : parsed
}

const toTransfer = (row: ResultRow): TransferRow => ({
  id: `t:${row.name}`,
  status: 'queued',
  name: row.name,
  user: row.user,
  progress: 'Queued',
  speed: '-',
  pct: 0,
  sizeMb: sizeMb(row.size),
  rateMbps: 1.4 + (row.name.length % 7) * 0.4,
})

export const HeroTerminal = () => {
  const [searches, setSearches] = useState<readonly SearchRow[]>(SEARCHES)
  const [results, setResults] =
    useState<Readonly<Record<string, readonly ResultRow[]>>>(RESULTS)
  const [activeId, setActiveId] = useState('s1')
  const [transfers, setTransfers] = useState<readonly TransferRow[]>(TRANSFERS)
  const [focus, setFocus] = useState(2)

  const [phase, setPhase] = useState<Phase>('searching')
  const [selected, setSelected] = useState(0)

  const shellRef = useRef<HTMLDivElement>(null)

  const resultsRef = useRef(results)
  useEffect(() => {
    resultsRef.current = results
  }, [results])

  const rows = useMemo(() => results[activeId] ?? [], [results, activeId])
  const activeSearch = searches.find((search) => search.id === activeId)

  const counts = useMemo<StatusCounts>(() => {
    const tally: StatusCounts = { active: 0, done: 0, fail: 0, queued: 0 }
    for (const transfer of transfers) tally[transfer.status] += 1
    return tally
  }, [transfers])

  const pct = useMemo(() => {
    const live = transfers.filter(
      (t) => t.pct !== undefined && (t.status === 'active' || t.status === 'done'),
    )
    if (live.length === 0) return 0
    return Math.round(live.reduce((sum, t) => sum + (t.pct ?? 0), 0) / live.length)
  }, [transfers])

  const hasWork = useMemo(
    () => transfers.some((t) => t.pct !== undefined && t.status !== 'done'),
    [transfers],
  )

  useEffect(() => {
    if (!hasWork || prefersReducedMotion()) return

    const timer = setInterval(() => {
      setTransfers((list) => {
        let live = list.filter((t) => t.status === 'active' && t.pct !== undefined).length

        const next = list.map((t) => {
          if (t.pct === undefined) return t
          const total = t.sizeMb ?? 12
          const rate = t.rateMbps ?? 2

          if (t.status === 'queued' && live < MAX_CONCURRENT) {
            live += 1
            return {
              ...t,
              status: 'active' as const,
              speed: `${rate.toFixed(2)} MB/s`,
              progress: `0.0/${total.toFixed(1)} MB (0%)`,
            }
          }
          if (t.status !== 'active') return t

          const step = 4 + ((t.name.length + Math.round(t.pct)) % 9)
          const pct = Math.min(100, t.pct + step)
          const finished = pct >= 100
          return {
            ...t,
            status: finished ? ('done' as const) : ('active' as const),
            pct,
            speed: finished ? '-' : `${rate.toFixed(2)} MB/s`,
            progress: finished
              ? 'Complete'
              : `${((total * pct) / 100).toFixed(1)}/${total.toFixed(1)} MB (${String(Math.round(pct))}%)`,
          }
        })

        const moved = next.some((t, i) => t !== list[i])
        return moved ? next : list
      })
    }, TICK_MS)

    return () => {
      clearInterval(timer)
    }
  }, [hasWork])

  const queueFrom = useCallback((searchId: string, indexes: readonly number[]) => {
    if (indexes.length === 0) return

    const current = resultsRef.current[searchId] ?? []
    const picked = indexes
      .map((i) => current[i])
      .filter((row): row is ResultRow => row !== undefined && row.queued !== true)
    if (picked.length === 0) return

    const wanted = new Set(picked.map((row) => row.name))
    setResults((prev) => ({
      ...prev,
      [searchId]: (prev[searchId] ?? []).map((row) =>
        wanted.has(row.name) ? { ...row, queued: true, marked: false } : row,
      ),
    }))
    setTransfers((list) => {
      const seen = new Set(list.map((transfer) => transfer.id))
      const additions = picked.map(toTransfer).filter((t) => !seen.has(t.id))
      return additions.length === 0 ? list : [...additions, ...list]
    })
  }, [])

  const queueIndexes = useCallback(
    (indexes: readonly number[]) => {
      queueFrom(activeId, indexes)
    },
    [queueFrom, activeId],
  )

  const toggleMark = useCallback(
    (index: number) => {
      setResults((prev) => ({
        ...prev,
        [activeId]: (prev[activeId] ?? []).map((row, i) =>
          i === index && row.queued !== true ? { ...row, marked: row.marked !== true } : row,
        ),
      }))
    },
    [activeId],
  )

  const markAll = useCallback(
    (on: boolean) => {
      setResults((prev) => ({
        ...prev,
        [activeId]: (prev[activeId] ?? []).map((row) =>
          row.queued === true ? row : { ...row, marked: on },
        ),
      }))
    },
    [activeId],
  )

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target instanceof HTMLInputElement) return
    const list = results[activeId] ?? []

    switch (event.key) {
      case 'ArrowDown':
      case 'j':
        event.preventDefault()
        setFocus(2)
        setSelected((s) => Math.min(list.length - 1, s + 1))
        return
      case 'ArrowUp':
      case 'k':
        event.preventDefault()
        setFocus(2)
        setSelected((s) => Math.max(0, s - 1))
        return
      case ' ':
        event.preventDefault()
        setFocus(2)
        toggleMark(selected)
        return
      case 'Enter': {
        event.preventDefault()
        const marked = list.flatMap((row, i) =>
          row.marked === true && row.queued !== true ? [i] : [],
        )
        queueIndexes(marked.length > 0 ? marked : [selected])
        setFocus(3)
        return
      }
      case 'a':
        event.preventDefault()
        markAll(true)
        return
      case 'A':
        event.preventDefault()
        markAll(false)
        return
      case '1':
      case '2':
      case '3':
        event.preventDefault()
        setFocus(Number(event.key))
        return
      default:
        return
    }
  }

  useEffect(() => {
    const timers: ReturnType<typeof setTimeout>[] = []
    const at = (fn: () => void, ms: number) => timers.push(setTimeout(fn, ms))
    const cancel = () => {
      for (const timer of timers) clearTimeout(timer)
    }

    if (prefersReducedMotion()) {
      at(() => {
        setSelected(RESTING_ROW)
        setPhase('interactive')
      }, 0)
      return cancel
    }

    const mark = (i: number) => {
      setResults((prev) => ({
        ...prev,
        s1: (prev['s1'] ?? []).map((row, ix) => (ix === i ? { ...row, marked: true } : row)),
      }))
    }

    at(() => {
      setPhase('results')
    }, 1050)
    at(() => {
      setSelected(3)
    }, 1550)
    at(() => {
      setSelected(7)
    }, 2050)
    at(() => {
      setSelected(10)
    }, 2550)
    at(() => {
      mark(10)
    }, 2850)
    at(() => {
      queueFrom('s1', [10])
    }, 3050)
    at(() => {
      setSelected(3)
    }, 3950)
    at(() => {
      mark(3)
    }, 4150)
    at(() => {
      queueFrom('s1', [3])
    }, 4350)
    at(() => {
      setSelected(RESTING_ROW)
      setPhase('interactive')
    }, 5050)

    return cancel
  }, [queueFrom])

  const submitSearch = (query: string) => {
    const id = `s${String(searches.length + 1)}`
    setSearches((prev) => [
      ...prev,
      { id, query, results: 40 + Math.floor(Math.random() * 900), status: 'Done' },
    ])
    setResults((prev) => ({
      ...prev,
      [id]: Array.from({ length: 6 }, (_, i) => ({
        name: `shared\\results\\${query.replace(/[^a-z0-9]+/gi, '_')}\\0${String(i + 1)} - track.flac`,
        size: `${(10 + i * 7).toFixed(1)} MB`,
        user: ['loopdigger', 'tapehiss', 'nectoUA'][i % 3] ?? 'loopdigger',
        bitrate: `${String(500 + i * 60)} kbps`,
        speed: '2.20 MB/s',
        slots: i % 2,
      })),
    }))
    setActiveId(id)
    setSelected(0)
    setFocus(2)
    setPhase('interactive')
  }

  const { wrapRef, innerRef, scale, height, ready } = useFitScale(DESIGN_WIDTH)

  const shownSearches = searches.map((search, i) =>
    i === 0 && phase === 'searching' ? { ...search, status: '⋯', results: '—' } : search,
  )
  const shownRows = phase === 'searching' ? [] : rows

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
              ref={shellRef}
              tabIndex={0}
              role="application"
              aria-label="Interactive soulseek-rs terminal demo"
              onKeyDown={onKeyDown}
              className="tui-content p-2.5 outline-none"
            >
              <StatusLine counts={counts} pct={pct} />

              <div className="mt-3 grid gap-2.5" style={{ gridTemplateColumns: '300px 1fr' }}>
                <div className="min-h-[498px]">
                  <SearchesPane
                    searches={shownSearches}
                    activeId={activeId}
                    active={focus === 1}
                    onPick={(id) => {
                      setActiveId(id)
                      setSelected(0)
                      setFocus(1)
                      setPhase('interactive')
                    }}
                    onSubmit={submitSearch}
                  />
                </div>

                <div className="grid min-h-0 gap-2.5" style={{ gridTemplateRows: '1fr 300px' }}>
                  <div className="relative min-h-[250px] overflow-hidden">
                    <ResultsPane
                      query={activeSearch?.query ?? ''}
                      rows={shownRows}
                      selected={selected}
                      active={focus === 2}
                      onSelect={(i) => {
                        setSelected(i)
                        setFocus(2)
                      }}
                      onQueue={(i) => {
                        queueIndexes([i])
                        setFocus(3)
                      }}
                    />
                    {phase === 'searching' && (
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
                      <TransfersPane rows={transfers.slice(0, 7)} active={focus === 3} />
                    </div>
                    <div className="min-h-0 min-w-0 overflow-hidden">
                      <InfoPane row={phase === 'searching' ? undefined : rows[selected]} />
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
