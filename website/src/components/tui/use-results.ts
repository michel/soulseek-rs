import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { RESULTS, type ResultRow } from './data'

type Results = Readonly<Record<string, readonly ResultRow[]>>

/** Result sets per search, which row is selected, and what is marked or already queued. */
export const useResults = (queue: (rows: readonly ResultRow[]) => void) => {
  const [results, setResults] = useState<Results>(RESULTS)
  const [activeId, setActiveId] = useState('s1')
  const [selected, setSelected] = useState(0)

  const resultsRef = useRef(results)
  useEffect(() => {
    resultsRef.current = results
  }, [results])

  const rows = useMemo(() => results[activeId] ?? [], [results, activeId])

  const patch = useCallback(
    (searchId: string, edit: (row: ResultRow, index: number) => ResultRow) => {
      setResults((prev) => ({ ...prev, [searchId]: (prev[searchId] ?? []).map(edit) }))
    },
    [],
  )

  const queueFrom = useCallback(
    (searchId: string, indexes: readonly number[]) => {
      const current = resultsRef.current[searchId] ?? []
      const picked = indexes
        .map((i) => current[i])
        .filter((row): row is ResultRow => row !== undefined && row.queued !== true)
      if (picked.length === 0) return

      const wanted = new Set(picked.map((row) => row.name))
      patch(searchId, (row) =>
        wanted.has(row.name) ? { ...row, queued: true, marked: false } : row,
      )
      queue(picked)
    },
    [patch, queue],
  )

  const mark = useCallback(
    (row: number) => {
      patch('s1', (r, i) => (i === row ? { ...r, marked: true } : r))
    },
    [patch],
  )

  const open = useCallback((id: string, next?: readonly ResultRow[]) => {
    if (next) setResults((prev) => ({ ...prev, [id]: next }))
    setActiveId(id)
    setSelected(0)
  }, [])

  return {
    rows,
    activeId,
    selected,
    setSelected,
    open,
    mark,
    queueFrom,
    queueIndexes: (indexes: readonly number[]) => {
      queueFrom(activeId, indexes)
    },
    toggleMark: (index: number) => {
      patch(activeId, (row, i) =>
        i === index && row.queued !== true ? { ...row, marked: row.marked !== true } : row,
      )
    },
    markAll: (on: boolean) => {
      patch(activeId, (row) => (row.queued === true ? row : { ...row, marked: on }))
    },
  }
}
