import { useState, type SyntheticEvent } from 'react'

import { cn } from '@/lib/utils'

import { TuiPane } from './chrome'
import type { ResultRow, SearchRow, TransferRow, TransferStatus } from './data'

const RESULT_COLS = '30px 1fr 74px 92px 78px 74px 42px'
const TRANSFER_COLS = '48px 1fr 96px 150px 74px'

interface SearchesPaneProps {
  searches: readonly SearchRow[]
  activeId: string
  active: boolean
  onPick: (id: string) => void
  onSubmit: (query: string) => void
}

export const SearchesPane = ({
  searches,
  activeId,
  active,
  onPick,
  onSubmit,
}: SearchesPaneProps) => {
  const [query, setQuery] = useState('')

  const submit = (event: SyntheticEvent<HTMLFormElement, SubmitEvent>) => {
    event.preventDefault()
    const trimmed = query.trim()
    if (trimmed === '') return
    onSubmit(trimmed)
    setQuery('')
  }

  return (
    <TuiPane
      title="Searches"
      num={1}
      active={active}
      className="h-full"
      bodyClassName="px-3 pt-4 pb-3"
    >
      <form
        onSubmit={submit}
        className="mb-3 flex items-center gap-1.5 border-b border-[color-mix(in_srgb,var(--color-dust)_25%,transparent)] pb-2.5"
      >
        <span className="text-[13px] text-[var(--color-oxide)]">/</span>
        <input
          value={query}
          onChange={(event) => {
            setQuery(event.target.value)
          }}
          placeholder="search the network…"
          aria-label="Search the network"
          className="grow border-0 bg-transparent font-mono text-[13px] text-[var(--color-paper)] outline-none placeholder:text-[var(--color-dust)]"
        />
        <span className="h-[15px] w-2 rounded-[1.5px] bg-[var(--color-oxide)] motion-safe:animate-cursor-blink" />
      </form>

      <div
        className="grid px-1 pb-1.5 text-xs text-[var(--color-dust)]"
        style={{ gridTemplateColumns: '56px 1fr 58px' }}
      >
        <span>Status</span>
        <span>Query</span>
        <span className="text-right">Results</span>
      </div>

      {searches.map((search) => (
        <button
          type="button"
          key={search.id}
          onClick={() => {
            onPick(search.id)
          }}
          className={cn(
            'grid w-full cursor-pointer p-1 text-left text-[13px] text-[var(--color-paper)]',
            search.id === activeId && 'bg-[color-mix(in_srgb,var(--color-signal)_14%,transparent)]',
          )}
          style={{ gridTemplateColumns: '56px 1fr 58px' }}
        >
          <span className="text-[var(--color-phosphor)]">
            {search.id === activeId ? '→' : ' '}
            {search.status}
          </span>
          <span className="overflow-hidden text-ellipsis whitespace-nowrap">
            {search.query}
          </span>
          <span className="text-right text-[var(--color-dust)]">{search.results}</span>
        </button>
      ))}
    </TuiPane>
  )
}

const resultGlyph = (row: ResultRow): { glyph: string; tone: string } => {
  if (row.queued === true) return { glyph: '✓', tone: 'text-[var(--color-phosphor)]' }
  if (row.marked === true) return { glyph: '[✓]', tone: 'text-[var(--color-oxide)]' }
  return { glyph: '[ ]', tone: 'text-[var(--color-dust)]' }
}

interface ResultsPaneProps {
  query: string
  rows: readonly ResultRow[]
  selected: number
  active: boolean
  onSelect: (index: number) => void
  onQueue: (index: number) => void
}

export const ResultsPane = ({
  query,
  rows,
  selected,
  active,
  onSelect,
  onQueue,
}: ResultsPaneProps) => (
  <TuiPane
    title={`Results: ${query}`}
    num={2}
    active={active}
    className="flex h-full flex-col"
    bodyClassName="flex min-h-0 flex-1 flex-col pt-4"
  >
    <div
      className="grid gap-2 border-b border-[color-mix(in_srgb,var(--color-dust)_25%,transparent)] px-3.5 pb-2 text-xs text-[var(--color-dust)]"
      style={{ gridTemplateColumns: RESULT_COLS }}
    >
      <span>✓</span>
      <span>Filename</span>
      <span>Size</span>
      <span>User</span>
      <span>Bitrate</span>
      <span>Speed</span>
      <span>Slots</span>
    </div>
    <div className="flex-1 overflow-y-auto">
      {rows.map((row, i) => {
        const { glyph, tone } = resultGlyph(row)
        const isSelected = selected === i
        return (
          <button
            type="button"
            key={row.name}
            onClick={() => {
              onSelect(i)
            }}
            onDoubleClick={() => {
              onQueue(i)
            }}
            className={cn(
              'grid w-full cursor-pointer gap-2 px-3.5 py-[3px] text-left text-[12.5px] text-[var(--color-paper)]',
              isSelected && 'bg-[color-mix(in_srgb,var(--color-oxide)_16%,transparent)]',
            )}
            style={{ gridTemplateColumns: RESULT_COLS }}
          >
            <span className={cn('whitespace-nowrap', tone)}>{glyph}</span>
            <span
              className={cn(
                'overflow-hidden text-ellipsis whitespace-nowrap',
                isSelected ? 'text-[var(--color-paper)]' : 'text-[var(--color-paper-dim)]',
              )}
            >
              {row.name}
            </span>
            <span className="text-right text-[var(--color-tape)]">{row.size}</span>
            <span className="text-[var(--color-signal)]">{row.user}</span>
            <span className="text-[var(--color-dust)]">{row.bitrate}</span>
            <span className="text-[var(--color-dust)]">{row.speed}</span>
            <span className="text-center text-[var(--color-dust)]">{row.slots}</span>
          </button>
        )
      })}
    </div>
  </TuiPane>
)

const TRANSFER_GLYPH: Record<TransferStatus, readonly [string, string]> = {
  queued: ['⋯', 'text-[var(--color-tape)]'],
  done: ['✓', 'text-[var(--color-phosphor)]'],
  active: ['↯', 'text-[var(--color-oxide)]'],
  fail: ['✗', 'text-[var(--color-alarm)]'],
}

const PROGRESS_TONE: Record<TransferStatus, string> = {
  queued: 'text-[var(--color-paper)]',
  active: 'text-[var(--color-paper)]',
  done: 'text-[var(--color-phosphor)]',
  fail: 'text-[var(--color-alarm)]',
}

interface TransfersPaneProps {
  rows: readonly TransferRow[]
  active: boolean
}

export const TransfersPane = ({ rows, active }: TransfersPaneProps) => (
  <TuiPane
    title="Downloads/Uploads"
    num={3}
    active={active}
    className="flex h-full flex-col"
    bodyClassName="flex min-h-0 flex-1 flex-col pt-4"
  >
    <div
      className="grid gap-2 border-b border-[color-mix(in_srgb,var(--color-dust)_25%,transparent)] px-3.5 pb-2 text-xs text-[var(--color-dust)]"
      style={{ gridTemplateColumns: TRANSFER_COLS }}
    >
      <span>Status</span>
      <span>Filename</span>
      <span>User</span>
      <span>Progress</span>
      <span>Speed</span>
    </div>
    <div className="flex-1 overflow-y-auto">
      {rows.map((row) => {
        const [glyph, tone] = TRANSFER_GLYPH[row.status]
        return (
          <div
            key={row.id}
            className="grid gap-2 px-3.5 py-[3px] text-[12.5px] text-[var(--color-paper-dim)]"
            style={{ gridTemplateColumns: TRANSFER_COLS }}
          >
            <span className={tone}>{glyph}</span>
            <span className="overflow-hidden text-ellipsis whitespace-nowrap">
              {row.name}
            </span>
            <span className="text-[var(--color-signal)]">{row.user}</span>
            <span className={PROGRESS_TONE[row.status]}>{row.progress}</span>
            <span className="text-[var(--color-tape)]">{row.speed}</span>
          </div>
        )
      })}
    </div>
  </TuiPane>
)

interface InfoPaneProps {
  row: ResultRow | undefined
}

export const InfoPane = ({ row }: InfoPaneProps) => {
  const parts = (row?.name ?? '').split('\\')
  const file = parts.at(-1) ?? '—'
  const path = parts.slice(0, -1).join('/')
  const queued = row?.queued === true

  const fields: readonly (readonly [string, string, string])[] = [
    ['User', row?.user ?? '—', 'text-[var(--color-signal)]'],
    ['Size', row?.size ?? '—', 'text-[var(--color-tape)]'],
    [
      'Status',
      queued ? 'Queued' : 'Available',
      queued ? 'text-[var(--color-tape)]' : 'text-[var(--color-phosphor)]',
    ],
    ['Bitrate', row?.bitrate ?? '-', 'text-[var(--color-paper)]'],
  ]

  return (
    <TuiPane title="Info" active className="h-full" bodyClassName="px-3.5 pt-4 pb-3.5">
      <div className="mb-1 text-[13.5px] break-all text-[var(--color-oxide)]">{file}</div>
      <div className="mb-4 text-xs break-all text-[var(--color-dust)]">
        {path === '' ? '—' : path}
      </div>
      <div
        className="grid gap-y-[7px] text-[12.5px]"
        style={{ gridTemplateColumns: '88px 1fr' }}
      >
        {fields.map(([key, value, tone]) => (
          <div key={key} className="contents">
            <span className="text-[var(--color-dust)]">{key}</span>
            <span className={cn('break-all', tone)}>{value}</span>
          </div>
        ))}
        <span className="mt-2.5 text-[var(--color-dust)]">Queue position</span>
        <span className="mt-2.5 text-[var(--color-paper)]">{queued ? '3' : 'unknown'}</span>
        <span className="text-[var(--color-dust)]">Free slots</span>
        <span className="text-[var(--color-phosphor)]">
          {row !== undefined && row.slots > 0 ? `${String(row.slots)} available` : '0'}
        </span>
      </div>
    </TuiPane>
  )
}
