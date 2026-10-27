# ADR-0007: Linux isolation uses an owned launch boundary

Date: 2026-10-27. Status: **accepted direction; implementation pending**.
The user selected the owned bootstrap over a util-linux runtime dependency.

## Evidence

[The return survey](../findings/2026-10-27-spec-007-return-survey.md)
checked the production launcher, existing platform modules, legacy donor, and
actual WSL namespace/cgroup privileges before considering a new component.

The modules do not provide a correct session launch lifecycle. In particular,
`unshare(CLONE_NEWPID)` affects future children, not the process about to
`exec`. Running namespace setup on a daemon worker would mutate the wrong
process. A post-fork callback must not allocate or acquire inherited locks.
The seccomp allowlist also needs an ABI-correct audit, and Linux capping
cannot be obtained from the macOS-only rlimit implementation.

## Decision

Use an owned Linux bootstrap process, implemented in `malt-platform` with
only its OS operations there. A CLI entry point would dispatch to it before
starting daemon threads. Do not add a live sibling dependency or assume
util-linux tools constitute a MALT containment backend.

The amended Spec 007 plan must implement and test:

- A setup/exec acknowledgement that cannot report success before namespace,
  privilege, mount and resource enforcement have all succeeded. Any
  inter-component handshake remains typed VNP, not an ad-hoc control protocol.
- The PID-namespace init/reaper and signal/exit propagation lifecycle,
  including killing descendants if the owning daemon/launch process dies.
- Rootless namespace availability and explicit cgroup delegation. The normal
  WSL user currently cannot write the root cgroup hierarchy; lack of
  delegation must refuse Capped, never silently drop limits or run as root.
- Filesystem roots, mount propagation, capabilities, network isolation and an
  architecture-correct seccomp policy for Contained, not just process grouping.
- Application to both MASH external commands and Compat PTY processes.
- Live adjacent-tier, failed-setup cleanup, loss, restore and daemon-death tests.

## Alternatives

An explicit dependency on util-linux `unshare` could supply the fork/init
portion, but adds packaging/runtime requirements and does not itself supply
cgroup delegation, containment configuration or MALT lifecycle integration.
It needs an explicit product/deployment decision rather than an undeclared
fallback when the owned bootstrap is absent.

Blindly calling the existing functions in `pre_exec` is rejected: it would
make the tests compile while leaving the promised PID boundary incorrect.

## Scope and current behavior

This is larger than wiring existing modules, as T032 originally assumed.
Keep T032 open until the amended launch-boundary tasks and their live tests pass.
Linux Required remains an explicit refusal; Preferred remains a visible
Bare downgrade. No new Linux enforcement or support level is claimed here.
