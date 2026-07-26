import { useCallback, useEffect, useMemo, useState } from 'react'

import type { StatusCounts } from './chrome'
import { TRANSFERS, type ResultRow, type TransferRow } from './data'

const TICK_MS = 480
const MAX_CONCURRENT = 2

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

const begin = (t: TransferRow, total: number, rate: number): TransferRow => ({
  ...t,
  status: 'active',
  speed: `${rate.toFixed(2)} MB/s`,
  progress: `0.0/${total.toFixed(1)} MB (0%)`,
})

const advance = (t: TransferRow, total: number, rate: number): TransferRow => {
  const from = t.pct ?? 0
  const pct = Math.min(100, from + 4 + ((t.name.length + Math.round(from)) % 9))
  const finished = pct >= 100
  return {
    ...t,
    status: finished ? 'done' : 'active',
    pct,
    speed: finished ? '-' : `${rate.toFixed(2)} MB/s`,
    progress: finished
      ? 'Complete'
      : `${((total * pct) / 100).toFixed(1)}/${total.toFixed(1)} MB (${String(Math.round(pct))}%)`,
  }
}

const tick = (list: readonly TransferRow[]): readonly TransferRow[] => {
  let live = list.filter((t) => t.status === 'active' && t.pct !== undefined).length

  const next = list.map((t) => {
    if (t.pct === undefined) return t
    const total = t.sizeMb ?? 12
    const rate = t.rateMbps ?? 2

    if (t.status === 'queued' && live < MAX_CONCURRENT) {
      live += 1
      return begin(t, total, rate)
    }
    return t.status === 'active' ? advance(t, total, rate) : t
  })

  return next.some((t, i) => t !== list[i]) ? next : list
}

export const useTransfers = (animate: boolean) => {
  const [transfers, setTransfers] = useState<readonly TransferRow[]>(TRANSFERS)

  const hasWork = useMemo(
    () => transfers.some((t) => t.pct !== undefined && t.status !== 'done'),
    [transfers],
  )

  useEffect(() => {
    if (!hasWork || !animate) return

    const timer = setInterval(() => {
      setTransfers(tick)
    }, TICK_MS)

    return () => {
      clearInterval(timer)
    }
  }, [hasWork, animate])

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

  const queue = useCallback((rows: readonly ResultRow[]) => {
    setTransfers((list) => {
      const seen = new Set(list.map((transfer) => transfer.id))
      const additions = rows.map(toTransfer).filter((t) => !seen.has(t.id))
      return additions.length === 0 ? list : [...additions, ...list]
    })
  }, [])

  return { transfers, counts, pct, queue }
}
