# Concurrency benchmark

Answers one question: how much simultaneous Soulseek activity can the client
absorb before it starts losing work?

```sh
scripts/stress.sh           # run the default profile
scripts/stress.sh --save    # ...and store it as the new baseline if it wins
```

A run takes about 7 seconds. It spawns a local soulfind, surrounds one real
`Client` with a swarm of mock peers, and drives searches, downloads, uploads and
browses **at the same time** — the mixing is the point, since every ceiling found
so far only appeared under a mixed load.

## The score

One number out of 100, weighted across five dimensions:

| Dimension  | Weight | Measures                                        |
|------------|--------|-------------------------------------------------|
| downloads  | 0.30   | completed / started                             |
| uploads    | 0.20   | served / requested                              |
| searches   | 0.20   | queries that got at least one result            |
| browses    | 0.15   | browse requests answered                        |
| throughput | 0.15   | aggregate MiB/s against `TARGET_MBPS` (500)     |

The four functional dimensions should stay at 100%: anything less means work was
silently dropped, which is the failure mode that matters. Throughput is the term
with headroom, so the score still moves when transfer handling regresses.

`--save` only overwrites `stress-baseline.json` when the run beats it, so the
baseline ratchets upward. Without `--save` a run just prints the delta.

## Tuning the load

Every knob is an env var; the defaults are in `Config::from_env`.

```sh
STRESS_PEERS=2048 STRESS_FILE_KB=8192 scripts/stress.sh
```

`STRESS_PEERS` `STRESS_SEARCHES` `STRESS_DOWNLOADS` `STRESS_LEECHERS`
`STRESS_BROWSES` `STRESS_FILE_KB` `STRESS_TIMEOUT`

## Reading a failure

`downloads completed 480/512` is the headline, but the two lines under it say
where to look:

- **`download states`** — a straggler on `Queued` never got its transfer
  negotiated; one on `InProgress` is merely slow. These are different bugs.
- **`mocks never online`** — mock peers the harness could not bring up. That is
  a harness/server limit, not the client, and it is reported separately so it
  cannot quietly shrink a denominator and flatter the score.

`peak threads` counts the whole process — client *and* mock swarm — so treat it
as a trend, not a client measurement.

`LOG_LEVEL=debug scripts/stress.sh` correlates a stuck peer's name against the
client's own trace, which is how each ceiling below was actually found.

## Ceilings found and removed

| Symptom                                            | Cause                                                                         |
|----------------------------------------------------|-------------------------------------------------------------------------------|
| Hard stop at ~15 peers; later peers never start    | Every `PeerActor` held a `ThreadPool` worker for life, so the pool capped peers at one per core |
| Downloads stuck on `Queued` under load             | `TransferResponse` was sent before the client recorded the peer's token, so the file connection raced its own bookkeeping |
| One download vanishing per few hundred             | Token was `md5(filename)`, and the store deletes *every* entry with a token — same filename from two peers destroyed both |
| Uploads dropped after the peer accepted            | Server answered `GetPeerAddress` with port 0; the client cached it and dialled it |

## Requirements

A soulfind binary (`SOULFIND_BIN`, defaulted in `scripts/stress.sh`), or
`SOULSEEK_TEST_SERVER=host:port` to point at a running server. The script raises
the descriptor limit — a swarm holds a lot of sockets open.
