# On-Call Solver
**Automatically compute a reasonable on-call schedule for your team using a declarative constraint-based approach.**

This tool is designed to help teams with a reasonably complex on-call schedule quickly and consistently
generate a reasonable, fair, schedule without the manual legwork of needing to manually handle exceptions
for holidays, sick-days etc.

## Features
 - **Supports Variable On-Call Schedules** which can have arbitrary cycle lengths and constraints like which days of
   the week they cover, and which hours of the day they run for.
 - **Allows for complex availability constraints** which can be applied to individual engineers, including
   periods of unavailability, days of the week they are unable to cover, etc.
 - **Fairness** is computed as the amount of time that a given engineer is on-call, and the tool will attempt
   to generate a schedule that is as fair as possible at any given point in time (see the [Factors](#factors) section for more info).
 - **Manages Workload and Recovery** by ensuring that engineers don't, wherever possible, perform back-to-back shifts and that they
   get time between shifts to recover (while ensuring fairness in the face of holidays etc).
 - **Stable Output** ensures that you can run the tool multiple times and receive the same output each time, allowing it to be
   used incrementally without impacting the schedule unless changes need to be made for new constraints.

## Usage

```bash
$ on-call --config .\examples\3-day.yaml --start 2023-01-01 --end 2023-12-30 --debug --format json
```

### Output Formats
You can specify the output format using the `--format` flag. The following formats are supported:

 - `human` - Outputs the schedule as a human-readable list of shifts
 - `json` - Outputs the schedule as a JSON object
 - `csv` - Outputs the schedule as a CSV file
 - `none` - Outputs only the statistics about the schedule (useful for verifying fairness)

## How It Works
The tool works by generating a sequence of time slots that a given on-call rotation needs to fill and incrementally comparing this
against the availability and cost of placing each engineer on-call for that shift slot. We take into consideration a range of
[Factors](#factors) to determine this cost function, including the amount of time an engineer has been on-call relative to the rest
of the team, how recently they were on-call, and whether they will be able to cover the full shift length. These allow us to then
select the engineer best positioned to cover the shift, and then repeat the process for the next shift slot.

**NOTE** Because this is a forward-only algorithm, it does not guarantee optimality and will not (for example) schedule a suboptimal
engineer for a shift to ensure better optimality for a future shift.

### Factors

#### Shift Length
This factor is used to try and ensure that engineers do not cover back-to-back shifts, as this is a common source of burnout and
anxiety. If an engineer was the most recent on-call for a schedule, they will be assigned a substantially higher cost, ensuring that
they are only placed on-call if there is no alternative.

#### Workload Fairness
This factor is computed as the amount of time that a given engineer has been on-call relative to the person on the team who has
the lowest amount of on-call time. By attaching a higher cost to placing a given engineer on-call when they have already been on-call
for a longer period of time, we ensure that the workload is more evenly distributed across the team.

This particular factor helps greatly when engineers take time off, as it ensures that the engineer will make up the time when they
return to work (assuming they are available to cover the shift, and without violating the other constraints).

#### Shift Coverage
We attempt to ensure that engineers are assigned to shifts that they are able to cover in their entirety wherever possible.
This constraint can, however, be violated if there is no better option available, for example if one of your engineers is unable
to cover on-call on Fridays (in which case they will be assigned on-call for the rest of the week and another engineer will be
assigned to cover that Friday).

#### Recency
Giving engineers time to recover between shifts is not only important to help manage burnout, it is also important to ensure that
they have time to focus on engineering work. We attempt to maximize the time between shifts for each engineer, assigning engineer
who have been off-call the longest before those who have been off-call for a shorter period of time (all other things being equal).

## Example
The tool requires that you specify your on-call rotation in a YAML file like the following. This file specifies the length of
your on-call rotation (which is the number of days that each engineer is on-call for), and a set of constraints that the schedule
must adhere to.

At the schedule level, your constraints determine the time slots that require on-call coverage, and will commonly restrict the
hours of the day that are to be covered, or the days of the week that require coverage - however you can also specify periods that
do not require on-call coverage if you wish.

At the human level, you can specify a set of constraints that apply to each engineer. These are most commonly used to declare the
time that the engineers are unavailable (due to planned leave), but can also be used to restrict the days of the week that they
will cover on-call (if you have part time employees, or people whose situations require them to be less available on certain days).

```yaml
shiftLength: 1 # A new shift starts every day
constraints:
  - !DayOfWeek [Mon, Tue, Wed, Thu, Fri] # Shifts cover weekdays only
  - !TimeRange # And run from 08:00 to 16:00 on those days
    start: 08:00:00
    end: 16:00:00
humans:
  alice@example.com:
    - !DayOfWeek [Mon, Wed, Fri] # Alice is only available on Mondays, Wednesdays and Fridays
  bob@example.com:
    - !Unavailable # Bob is taking vacation between these dates
      start: 2023-01-01
      end: 2023-01-07
  claire@example.com: # Claire has no availability restrictions
    - !None
```

```bash
# Run the on-call tool to generate a schedule from the start of December until March
$ on-call --config .\examples\3-day.yaml --start 2023-01-01 --end 2023-12-30
```

```
Humans:
  frank@example.com:
    - unavailable from 2023-02-09 to 2023-02-17
  donovan@example.com:
  alice@example.com:
    - available on Mon, Wed, Fri,
  claire@example.com:
    - always available
  bob@example.com:
    - unavailable from 2023-01-01 to 2023-01-07
  erica@example.com:

Workload: (min: 336, avg: 345, max: 360)
  frank@example.com: 360 hours
  donovan@example.com: 360 hours
  claire@example.com: 344 hours
  bob@example.com: 336 hours
  erica@example.com: 336 hours
  alice@example.com: 336 hours

Longest shift: (min: 8, avg: 21, max: 24)
  frank@example.com: 24 hours
  donovan@example.com: 24 hours
  bob@example.com: 24 hours
  claire@example.com: 24 hours
  erica@example.com: 24 hours
  alice@example.com: 8 hours

Shift length histogram:
 59 | 8 hours
 43 | 24 hours
 34 | 16 hours


Schedule:
  2023-01-06 08:00:00 - 2023-01-06 16:00:00: alice@example.com
  2023-01-09 08:00:00 - 2023-01-09 16:00:00: bob@example.com
  2023-01-10 08:00:00 - 2023-01-10 16:00:00: claire@example.com
  2023-01-11 08:00:00 - 2023-01-11 16:00:00: bob@example.com
  2023-01-12 08:00:00 - 2023-01-12 16:00:00: claire@example.com
  2023-01-13 08:00:00 - 2023-01-13 16:00:00: alice@example.com
  2023-01-16 08:00:00 - 2023-01-16 16:00:00: bob@example.com
  2023-01-17 08:00:00 - 2023-01-17 16:00:00: claire@example.com
  2023-01-18 08:00:00 - 2023-01-18 16:00:00: alice@example.com
  2023-01-19 08:00:00 - 2023-01-19 16:00:00: bob@example.com
  2023-01-20 08:00:00 - 2023-01-20 16:00:00: alice@example.com
  2023-01-23 08:00:00 - 2023-01-23 16:00:00: claire@example.com
  2023-01-24 08:00:00 - 2023-01-24 16:00:00: bob@example.com
  2023-01-25 08:00:00 - 2023-01-25 16:00:00: alice@example.com
  2023-01-26 08:00:00 - 2023-01-26 16:00:00: claire@example.com
  2023-01-27 08:00:00 - 2023-01-27 16:00:00: bob@example.com
  2023-01-30 08:00:00 - 2023-01-30 16:00:00: alice@example.com
  2023-01-31 08:00:00 - 2023-01-31 16:00:00: claire@example.com
  2023-02-01 08:00:00 - 2023-02-01 16:00:00: bob@example.com
  2023-02-02 08:00:00 - 2023-02-02 16:00:00: claire@example.com
```
