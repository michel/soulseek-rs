export interface SearchRow {
  id: string
  query: string
  results: number | string
  status: string
}

export interface ResultRow {
  name: string
  size: string
  user: string
  bitrate: string
  speed: string
  slots: number
  marked?: boolean
  queued?: boolean
}

export type TransferStatus = 'queued' | 'active' | 'done' | 'fail'

export interface TransferRow {
  id: string
  status: TransferStatus
  name: string
  user: string
  progress: string
  speed: string
  pct?: number
  sizeMb?: number
  rateMbps?: number
}

export const SHARE = '~/Downloads/Soulseek'

export const SEARCHES: readonly SearchRow[] = [
  { id: 's1', query: 'public domain field recordings', results: 606, status: 'Done' },
  { id: 's2', query: 'creative commons ambient', results: 33, status: 'Done' },
  { id: 's3', query: 'netlabel 2004', results: 608, status: 'Done' },
]

const pd = (name: string, size: string, bitrate: string): ResultRow => ({
  name: `shared\\archive\\PD Field Recordings\\${name}`,
  size,
  user: 'loopdigger',
  bitrate,
  speed: '3.35 MB/s',
  slots: 1,
})

export const RESULTS: Readonly<Record<string, readonly ResultRow[]>> = {
  s1: [
    pd('01 - Coastal Dawn (Sunrise Take).flac', '30.5 MB', '675 kbps'),
    pd('02 - Reservoir Wind.flac', '22.6 MB', '569 kbps'),
    pd('cover.jpg', '0.0 MB', '-'),
    pd('03 - Marsh at Night (Long).flac', '48.0 MB', '920 kbps'),
    pd('04 - Tram Depot, 06h.flac', '32.2 MB', '725 kbps'),
    pd('05 - Harbour Bells.flac', '52.5 MB', '902 kbps'),
    pd('06 - Rain on Zinc Roof.flac', '30.4 MB', '804 kbps'),
    pd('07 - Empty Chapel Tone.flac', '17.4 MB', '836 kbps'),
    pd('08 - Forest Floor.flac', '38.0 MB', '741 kbps'),
    pd('09 - Quarry Echo.flac', '34.6 MB', '753 kbps'),
    pd('10 - Night Ferry.flac', '18.7 MB', '723 kbps'),
    pd('11 - Last Train.flac', '19.5 MB', '621 kbps'),
    {
      name: 'shared\\netlabel\\Explorers of the Interior [CC-BY]\\readme.txt',
      size: '1.0 MB',
      user: 'loopdigger',
      bitrate: '128 kbps',
      speed: '3.35 MB/s',
      slots: 1,
    },
  ],
  s2: [
    {
      name: 'shared\\cc\\Ambient Works [CC0]\\01 - Slow Meadow.flac',
      size: '41.2 MB',
      user: 'tapehiss',
      bitrate: '911 kbps',
      speed: '2.10 MB/s',
      slots: 2,
    },
    {
      name: 'shared\\cc\\Ambient Works [CC0]\\02 - Held Note.flac',
      size: '28.9 MB',
      user: 'tapehiss',
      bitrate: '740 kbps',
      speed: '2.10 MB/s',
      slots: 2,
    },
    {
      name: 'shared\\cc\\Ambient Works [CC0]\\03 - Grain.flac',
      size: '33.1 MB',
      user: 'tapehiss',
      bitrate: '812 kbps',
      speed: '2.10 MB/s',
      slots: 2,
    },
  ],
  s3: [
    {
      name: 'shared\\netlabel\\2004 Comp [CC-BY-NC]\\01 - Opener.mp3',
      size: '9.4 MB',
      user: 'nectoUA',
      bitrate: '320 kbps',
      speed: '1.05 MB/s',
      slots: 0,
    },
    {
      name: 'shared\\netlabel\\2004 Comp [CC-BY-NC]\\02 - Interlude.mp3',
      size: '5.1 MB',
      user: 'nectoUA',
      bitrate: '320 kbps',
      speed: '1.05 MB/s',
      slots: 0,
    },
  ],
}

export const TRANSFERS: readonly TransferRow[] = [
  {
    id: 'x1',
    status: 'done',
    name: '@@lrfkq\\Contents\\Rotterdam',
    user: 'Dan Dillion',
    progress: 'Completed',
    speed: '-',
  },
  {
    id: 'x2',
    status: 'done',
    name: '@@lrfkq\\Contents\\Journal',
    user: 'Dan Dillion',
    progress: 'Completed',
    speed: '-',
  },
  {
    id: 'x3',
    status: 'active',
    name: '@@lrfkq\\Contents\\Context',
    user: 'Dan Dillion',
    progress: '37.5 MB/53.4 MB (70%)',
    speed: '12.4 MB/s',
  },
  {
    id: 'x4',
    status: 'fail',
    name: '@@gufoo\\put all musics',
    user: 'publicdomainbot',
    progress: 'Failed',
    speed: '-',
  },
  {
    id: 'x5',
    status: 'active',
    name: '02-RawMusic\\Embark Studio',
    user: 'tapehiss',
    progress: '10.4 MB/14.3 MB (72%)',
    speed: '11.5 MB/s',
  },
  {
    id: 'x6',
    status: 'fail',
    name: '@@medvw\\Soulseek\\complete',
    user: 'x100',
    progress: 'Failed',
    speed: '-',
  },
  {
    id: 'x7',
    status: 'active',
    name: '@@hnttf\\complete\\(2004 netlabel)',
    user: 'nectoUA',
    progress: '10.5 MB/25.5 MB (41%)',
    speed: '9.3 MB/s',
  },
]

export const SHORTCUTS: readonly (readonly [string, string])[] = [
  ['Space', 'select'],
  ['Enter', 'download'],
  ['b', 'browse owner'],
  ['c', 'chat'],
  ['/', 'filter'],
  ['a/A', 'select all/none'],
  ['1-3', 'focus pane'],
  ['q', 'quit'],
]
