# Contract: owned Linux session bootstrap

Accepted direction: 2026-10-27, ADR-0007. **Design, not delivered behavior.**

## Transport and identity

The daemon owns a private Unix socket pair and starts the bootstrap through
the early `malt` dispatcher with only the intended endpoint inherited. This
is not a discoverable pathname service and adds no system privilege. Use
schema-defined, bounded VNP frames; allocate domain/type constants only after
checking the existing schema and codec registries, and test those mappings.
The 2026-10-27 inventory found wire domains 0–7 assigned and 8–15 reserved
(`codec.rs` and the architecture domain table). No new ID is allocated by
this planning artifact; T044 must update schema, registry and tests together.
Handshake binds protocol version, nonce and session identity. Payloads must
not inherit the control descriptor or startup acknowledgement channels.

## Typed operations

- Configure/Ready/Refused: validated tier/resources, observed namespace and
  privilege/cgroup evidence, or a precise setup refusal with cleanup outcome.
- Launch/Started/Refused/Indeterminate: correlated request ID, exact argv0,
  args, cwd, environment and stdio/extra-fd/process-group/PTY specification.
  Started includes the payload's verified host identity, not the init PID.
  Exec failure is not Started; unknown delivery invalidates the carrier.
- Exit/Signal: owned payload identity, status/signal and acknowledged result.
  Foreign, exited or stale identities cannot target unrelated processes.
- Shutdown/Closed: reject new work, terminate/reap descendants and confirm
  release of owned mounts, cgroups and workspaces. Failure remains visible.

SCM_RIGHTS is used only for descriptor transport, with typed slot metadata
and exact cardinality/type/direction validation. Reject duplicates, extra or
missing descriptors and close them on all error paths. Never interpret the
sender's numeric fd as a receiver-owned descriptor.

## Admission and liveness

Ready requires the full requested tier, not a subset. Parent mapping and
cgroup placement complete before a child may execute untrusted code. Do not
allow a blocking Wait request to prevent concurrent launch, status, signal
or teardown. Control EOF/daemon death terminates the namespace; broker loss
or malformed/ambiguous replies invalidates the shared context. Every status
surface reports that same loss. No retry may silently select ordinary spawn.

## Configuration and capability

Optional daemon settings name a delegated cgroup subtree and Linux rootfs.
Neither is an arbitrary per-command path. Rootfs provisioning and a Linux
image registry are outside this contract. Missing resources produce a
discoverable Required refusal or explicit Preferred downgrade. Positive
capability claims require the actual configured setup/probe path to succeed;
no fallback to the host root, host network or absent CPU/memory enforcement.
