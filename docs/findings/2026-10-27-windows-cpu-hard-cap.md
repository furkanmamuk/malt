# Windows Capped had no CPU scheduler limit

Date: 2026-10-27. Discovered while reconciling Linux bootstrap limits with
the existing Windows policy. This is part of Spec 007's promised Capped
constraint, not a new tier or a new dependency.

## Before

`create_job_object` wrote `cpu_rate * 100_000` into
`per_job_user_time_limit`. That is not a CPU scheduler throttle. The matching
Job time-limit flag was also absent, so the value was ignored; it did **not**
impose a two-second lifetime deadline. An initial commentary inference about
termination was corrected after inspecting the flags.

The new real Win32 read-back test failed before the implementation change:
`ControlFlags` was 0, expected ENABLE | HARD_CAP (5). This establishes the
missing policy directly, not merely by inspecting a struct initializer.

## Repair

Use `JobObjectCpuRateControlInformation` with ENABLE | HARD_CAP and read back
both flags and rate. A native setup/read-back failure refuses the session.
The caller's units remain percent of one core: 100 means one core, 200 means
two. Convert to Windows' 1/10000 fraction of host capacity using the active
processor count; round toward a stronger cap, clamp at natural host capacity,
and refuse a below-resolution request instead of weakening it. Existing
nested job restrictions may make the resulting effective limit stronger.
The obsolete lifetime user-time field is zero. Partial setup now uses the
owned job handle's Drop rather than leaking an object on a CPU-policy error.

The native contract is documented by
[Microsoft's CPU-rate-control API](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_cpu_rate_control_information).

## Observed proof and limitations

The same two-second busy child consumed 1,953 ms process CPU without a limit
and 562 ms with a 20%-of-one-core cap. The workload/measurement happen in
separate child processes launched through the established session carrier.
The test permits scheduling-interval bursts and timer granularity, not
unlimited consumption. It reports an explicit limitation if competing host
load makes the unrestricted comparison uninformative; that did not occur in
this focused run. This is functional enforcement evidence, not a general
performance benchmark or an exact two-second utilization guarantee.

Rust 1.99 Windows workspace build, full tests, format and strict all-targets
Clippy passed after the repair. Full workspace tests included native Smoosh;
the separate launch/loss checkpoint had also explicitly reported 183 passed,
three unsupported skips, zero harness failures and zero shell failures.
Fresh privileged image/helper-loss gates remain outstanding for Spec 007.

Code checkpoint `e7c053d` also passed full WSL mirror build/tests and all
hosted jobs in [run 40521862119](https://github.com/furkanmamuk/malt/actions/runs/40521862119),
including actual Windows Smoosh. WSL's existing cfg/unused warnings remain;
this does not claim Linux strict-Clippy or Windows CPU enforcement on Linux.
