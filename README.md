# On-Call Solver
**Automatically compute a fair on-call schedule for your team using a declarative, constraint-based approach.**

This tool helps teams with a reasonably complex on-call schedule generate a fair rota without
the manual legwork of juggling holidays, part-time contracts, sick days and personal preferences
by hand.

It works by *optimizing*: it builds a candidate schedule, then spends a fixed budget of effort
repeatedly improving it, keeping the best schedule it finds. That lets it make trade-offs a
one-pass algorithm cannot — accepting a slightly awkward shift this week because it makes the
next three weeks considerably fairer.

## Installation
Install with [Homebrew](https://brew.sh):

```sh
brew install sierrasoftworks/tap/on-call
```

## Features
 - **Fairness that accounts for availability.** Everyone gets a target share of the on-call load,
   scaled to how much of the schedule they can actually cover. A part-time engineer is not
   expected to carry the same hours as a full-timer, and is not penalised for it either.
 - **Hard rules and soft preferences, kept separate.** Minimum rest and maximum shift length are
   enforced as rules. "I'd rather not have Mondays" is a preference the optimizer honours when it
   can afford to, and reports on when it cannot.
 - **Complex availability constraints** per engineer: planned leave, days of the week they cannot
   cover, restricted hours.
 - **Stable output.** The same inputs always produce the same schedule. Pass `--baseline` to also
   keep re-runs close to an already-published rota instead of reshuffling it.
 - **Honest reporting.** `--explain` shows exactly which objectives are costing what, and the
   summary reports each person's workload against their target rather than a bare average.

## Usage

```bash
$ on-call --config examples/3-day.yaml --start 2023-01-01 --end 2023-12-31
```

| Flag | Purpose |
|---|---|
| `--config <FILE>` | The YAML rota definition. |
| `--start`, `--end` | Horizon to schedule. Defaults to today through 28 days' time. `--end` is exclusive. |
| `--format` | `human` (default), `json`, `csv`, or `none` for statistics only. |
| `--baseline <FILE>` | A previously generated JSON schedule to stay close to. |
| `--freeze-before <DATE>` | Pin shifts ending before this date to their baseline owner. Requires `--baseline`. |
| `--steps <N>` | How many candidate changes to evaluate. Default 200,000. |
| `--seed <N>` | Optimizer seed. The same seed always gives the same schedule. |
| `--time-budget <SECONDS>` | Stop early after a wall-clock limit. Makes output machine-dependent, so off by default. |
| `--explain` | Print the score broken down by objective, plus optimizer statistics. |

The schedule goes to stdout; everything else goes to stderr, so `--format json > schedule.json`
gives you a clean file.

### Re-running without churning the roster

Once you have published a schedule, feed it back in so the optimizer only changes what it must:

```bash
# Generate and publish
$ on-call --config rota.yaml --start 2024-01-01 --end 2024-06-30 --format json > published.json

# Later: someone books leave. Re-plan, keeping the next two weeks fixed and
# minimising disruption to the rest.
$ on-call --config rota.yaml --start 2024-01-01 --end 2024-06-30 \
    --baseline published.json --freeze-before 2024-02-01 --format json > updated.json
```

Frozen shifts cannot move at all. Beyond the freeze date the optimizer starts from the published
schedule rather than from scratch, so it begins at zero disruption and only moves a shift when the
improvement clearly outweighs the churn. Re-running with nothing changed is a no-op, whatever seed
you use.

## How It Works

### 1. Building the problem
The schedule-level constraints are applied to your date range to work out which periods need
coverage. Those periods are then split at every point where *anybody's* availability changes, so
each resulting slot is either wholly coverable or wholly uncoverable by each person. This is what
lets the tool use somebody who is only free for half a day, instead of having to round their
availability up or down.

Each person is then given a target number of on-call hours. Targets always add up to exactly the
total time needing coverage, so a perfectly fair schedule is achievable and scores zero.

### 2. Scoring
Schedules are scored on two tiers, compared in order:

 - **Hard** — uncovered time, breaches of `minRestHours`, breaches of `maxConsecutiveHours`. Any
   schedule with fewer hard violations beats any schedule with more, whatever its soft score.
 - **Soft** — fairness, shift length, rest, preferences, and stability against a baseline.

Every soft objective reports its penalty in the same units, so the `weights` are directly
comparable to one another. Lower is better; a schedule that satisfies a lot of `prefer:`
preferences can score below zero.

Deviations are penalised quadratically rather than linearly. That makes one large unfairness
worse than several small ones, which is how teams actually experience it, and it gives the
optimizer a gradient to follow everywhere instead of a plateau to get stuck on.

### 3. Optimizing
An initial schedule is built greedily, then improved by late-acceptance hill climbing: a change is
kept if it beats either the current schedule or the one from a few hundred steps ago. That is
enough to climb out of local optima while still trending toward better schedules. When progress
stalls, a section of the schedule is torn out entirely and rebuilt, which reaches solutions that
no sequence of small changes could.

The optimizer only ever proposes people who are genuinely available, so availability is a property
of the search space rather than something the score has to police — it cannot be violated.

### Objectives

| Objective | What it does |
|---|---|
| `coverage` | *(hard)* Every slot should have somebody on it. |
| `fairness` | Everyone's hours should land on their target share. |
| `runLength` | Shifts should be about `shiftLength` long — neither fragmented nor punishing. Capped per person at the longest shift their availability permits, so part-timers are not priced out. |
| `rest` | People should get `desiredRestHours` between shifts. There is no reward for exceeding it, which stops one person being front-loaded and then never scheduled again. |
| `preference` | Honour `avoid:` and `prefer:` where affordable. |
| `stability` | Stay close to `--baseline`. |

## Configuration

Only `shiftLength` and `humans` are required; everything else has sensible defaults.

```yaml
# The nominal length of one shift, in days. A target, not a rule — the optimizer
# will accept a shorter or longer shift if it makes the schedule materially better.
shiftLength: 3

# Which periods need on-call coverage.
constraints:
  - !DayOfWeek [Mon, Tue, Wed, Thu, Fri]
  - !TimeOfDay
    start: 08:00:00
    end: 16:00:00

# Rules the optimizer treats as constraints rather than costs. All optional.
rules:
  minRestHours: 48         # minimum recovery time between two shifts
  maxConsecutiveHours: 72  # maximum on-call time within one unbroken shift
  desiredRestHours: 120    # rest to aim for; falling short costs, exceeding earns nothing

# Relative importance of each soft objective. Zero disables one entirely.
weights:
  fairness: 10
  runLength: 10
  rest: 3
  preference: 1
  stability: 5

# How each person's fair share is derived.
#   capacity (default) - proportional to how much of the rota they can cover
#   equal              - everyone carries the same hours regardless of availability
fairness: capacity

# Optional. Restricts handoffs to rotation boundaries, enforced structurally.
rotation:
  lock: true
  boundary: !DayOfWeek [Mon]   # or: !EverySlots 5

humans:
  alice@example.com:
    # Hard availability: Alice will never be scheduled outside these.
    constraints:
      - !DayOfWeek [Mon, Wed, Fri]

  bob@example.com:
    # Explicitly half time, regardless of what their availability implies.
    capacity: 0.5
    constraints:
      - !Unavailable
        start: 2023-01-01
        end: 2023-01-07   # exclusive

  claire@example.com:
    # Soft preferences: honoured where affordable, overridden where not.
    preferences:
      - avoid: !DayOfWeek [Mon]
        weight: 3
      - prefer: !TimeOfDay
          start: 08:00:00
          end: 12:00:00

  erica@example.com:
    # How far ahead of their fair share they already are. Reduces their target
    # until the team catches up. May be negative for somebody who is owed time.
    # The summary's `carry forward` column tells you what to put here next time.
    priorWorkload: 24
```

### Constraints

| Tag | Meaning |
|---|---|
| `!DayOfWeek [Mon, Wed]` | Only these days of the week. |
| `!TimeOfDay {start, end}` | Only these hours. `start` later than `end` wraps overnight. |
| `!Unavailable {start, end}` | Not during this date range. `end` is exclusive. |
| `!None` | No restriction. |

At the schedule level these determine which periods need coverage. At the person level they
determine when that person can be scheduled. The same tags work in `preferences`, where they
select the times a preference applies to.

## Example output

```
$ on-call --config examples/constrained.yaml --start 2023-01-01 --end 2023-06-30 --explain
```

```
Humans:
  alice@example.com: available on Mon, Wed, Fri
  bob@example.com: capacity: 0.5x
  claire@example.com: prefers not to be available on Mon (weight 3)
  donovan@example.com: prefers to be available on Fri (weight 2)
  erica@example.com: prior workload: 24 hours
  frank@example.com: unavailable from 2023-02-13 to 2023-02-20

Schedule: 129 slots covering 1032h, in shifts of about 16h
  rules: at least 48h rest between shifts, shifts no longer than 24h

Score: 0hard/-2830soft

Workload: (min: 104h, avg: 172h, max: 208h)
                                 actual   target    delta  carry forward
  alice@example.com                128h     124h      +3h  3h
  donovan@example.com              208h     208h       0h  0h
  claire@example.com               208h     208h       0h  0h
  erica@example.com                184h     184h       0h  0h
  frank@example.com                200h     200h       0h  0h
  bob@example.com                  104h     104h       0h  0h

Longest shift: (min: 16h, avg: 17h, max: 24h)
  alice@example.com              16h across 9 shifts, shortest rest 160h
  ...

Objectives:
  runLength    0hard/33600soft
  fairness     0hard/1010soft
  coverage     -
  rest         -
  stability    -
  preference   0hard/-37440soft
```

Read the workload table as: everyone landed on their target, except Alice who is three hours over
and should carry `priorWorkload: 3` into the next run. The `preference` objective is negative
because preferences were satisfied, not violated.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | A schedule was produced with every rule satisfied. |
| `1` | A schedule was produced, but it has coverage gaps or breaks a rule in `rules:`. Run with `--explain` to see which. |
| `2` | The configuration could not be read or is invalid. |

## Examples

| File | Demonstrates |
|---|---|
| `examples/rotation.yaml` | Daily handoffs. |
| `examples/3-day.yaml` | Three-day shifts, leave, carried-over workload. |
| `examples/weekly.yaml` | Week-long shifts. |
| `examples/constrained.yaml` | Hard rules, soft preferences, explicit capacity. |
| `examples/locked.yaml` | Rotations locked to Monday boundaries. |

## Using it as a library

The scheduler is also published as a library, so you can drive it directly:

```rust
use on_call::model::Problem;
use on_call::search::{self, Options};
use on_call::schedule::Schedule;

let problem = Problem::build(&config, start, end)?;
let (assignment, stats) = search::solve(&problem, &Options::default());
let schedule = Schedule::from_assignment(&problem, &assignment);
```
