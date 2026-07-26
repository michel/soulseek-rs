import { useCallback, useEffect, useState, type KeyboardEvent } from 'react'

import { SEARCHES, type ResultRow, type SearchRow } from './data'
import { useResults } from './use-results'
import { useTransfers } from './use-transfers'

const RESTING_ROW = 5

type Phase = 'searching' | 'results' | 'interactive'

const isBrowser = typeof window !== 'undefined'

const prefersReducedMotion = (): boolean =>
  isBrowser && window.matchMedia('(prefers-reduced-motion: reduce)').matches

const fakeResults = (query: string): readonly ResultRow[] =>
  Array.from({ length: 6 }, (_, i) => ({
    name: `shared\\results\\${query.replace(/[^a-z0-9]+/gi, '_')}\\0${String(i + 1)} - track.flac`,
    size: `${(10 + i * 7).toFixed(1)} MB`,
    user: ['loopdigger', 'tapehiss', 'nectoUA'][i % 3] ?? 'loopdigger',
    bitrate: `${String(500 + i * 60)} kbps`,
    speed: '2.20 MB/s',
    slots: i % 2,
  }))

interface Keys {
  rows: readonly ResultRow[]
  selected: number
  moveRow: (delta: number) => void
  focusPane: (pane: number) => void
  toggleMark: (index: number) => void
  markAll: (on: boolean) => void
  queueIndexes: (indexes: readonly number[]) => void
}

const handleKey = (k: Keys, event: KeyboardEvent<HTMLDivElement>) => {
  if (event.target instanceof HTMLInputElement) return

  switch (event.key) {
    case 'ArrowDown':
    case 'j':
      event.preventDefault()
      k.moveRow(1)
      return
    case 'ArrowUp':
    case 'k':
      event.preventDefault()
      k.moveRow(-1)
      return
    case ' ':
      event.preventDefault()
      k.focusPane(2)
      k.toggleMark(k.selected)
      return
    case 'Enter': {
      event.preventDefault()
      const marked = k.rows.flatMap((row, i) =>
        row.marked === true && row.queued !== true ? [i] : [],
      )
      k.queueIndexes(marked.length > 0 ? marked : [k.selected])
      k.focusPane(3)
      return
    }
    case 'a':
      event.preventDefault()
      k.markAll(true)
      return
    case 'A':
      event.preventDefault()
      k.markAll(false)
      return
    case '1':
    case '2':
    case '3':
      event.preventDefault()
      k.focusPane(Number(event.key))
      return
    default:
      return
  }
}

/** Plays the scripted intro once: the search resolves, rows walk, two downloads queue. */
const useIntroTimeline = (
  animate: boolean,
  rest: () => void,
  setPhase: (phase: Phase) => void,
  setSelected: (row: number) => void,
  mark: (row: number) => void,
  queueFrom: (searchId: string, indexes: readonly number[]) => void,
) => {
  useEffect(() => {
    const timers: ReturnType<typeof setTimeout>[] = []
    const at = (fn: () => void, ms: number) => timers.push(setTimeout(fn, ms))
    const cancel = () => {
      for (const timer of timers) clearTimeout(timer)
    }

    if (!animate) {
      at(rest, 0)
      return cancel
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
    at(rest, 5050)

    return cancel
  }, [animate, rest, setPhase, setSelected, mark, queueFrom])
}

export const useHeroDemo = () => {
  const [searches, setSearches] = useState<readonly SearchRow[]>(SEARCHES)
  const [focus, setFocus] = useState(2)
  const [phase, setPhase] = useState<Phase>('searching')

  const animate = !prefersReducedMotion()
  const { transfers, counts, pct, queue } = useTransfers(animate)
  const {
    rows,
    activeId,
    selected,
    setSelected,
    open,
    mark,
    queueFrom,
    queueIndexes,
    toggleMark,
    markAll,
  } = useResults(queue)

  const rest = useCallback(() => {
    setSelected(RESTING_ROW)
    setPhase('interactive')
  }, [setSelected])

  useIntroTimeline(animate, rest, setPhase, setSelected, mark, queueFrom)

  const searching = phase === 'searching'

  const moveRow = (delta: number) => {
    setFocus(2)
    setSelected((s) => Math.min(rows.length - 1, Math.max(0, s + delta)))
  }

  const submitSearch = (query: string) => {
    const id = `s${String(searches.length + 1)}`
    const results = 40 + Math.floor(Math.random() * 900)
    setSearches((prev) => [...prev, { id, query, results, status: 'Done' }])
    open(id, fakeResults(query))
    setFocus(2)
    setPhase('interactive')
  }

  return {
    searches: searches.map((search, i) =>
      i === 0 && searching ? { ...search, status: '⋯', results: '—' } : search,
    ),
    rows: searching ? [] : rows,
    infoRow: searching ? undefined : rows[selected],
    query: searches.find((search) => search.id === activeId)?.query ?? '',
    activeId,
    transfers,
    counts,
    pct,
    focus,
    selected,
    searching,
    submitSearch,
    pickSearch: (id: string) => {
      open(id)
      setFocus(1)
      setPhase('interactive')
    },
    selectRow: (i: number) => {
      setSelected(i)
      setFocus(2)
    },
    queueRow: (i: number) => {
      queueIndexes([i])
      setFocus(3)
    },
    onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => {
      handleKey(
        { rows, selected, moveRow, focusPane: setFocus, queueIndexes, toggleMark, markAll },
        event,
      )
    },
  }
}
