# Gateway hardening: implementation and evidence

Date: 2026-10-27. Baseline: `9b3e327` on Windows.

## Survey and correction to the plan

The limiter is reachable from the daemon's final `with_auth` router assembly,
before the scope check. It has no clock and no production reset caller. The
first story replaces the counter with monotonic fixed windows and reclaims
expired keys plus excess hash-table capacity on ordinary request access.

The body-limit premise in the July specification was too broad. Axum's
`Bytes`/`Json` extractors already default to a 2 MiB limit (`axum-core`'s
`extract/default_body_limit.rs` and `extract/request_parts.rs`). Absence of an
explicit layer did not establish unlimited JSON buffering. The remaining work
is an explicit, configurable, router-wide bound, pre-read Content-Length
refusal, streaming enforcement, and the structured error contract. This
correction preserves the original claim above as history rather than quietly
exchanging conclusions.

## US1: allowance recovery

- `authenticated_caller_recovers_and_another_token_is_unaffected`: real bearer
  tokens and the production auth middleware; exhaust A, refuse A, serve B,
  then serve A across four windows with no reset call.
- `elapsed_window_restores_allowance_without_a_reset_call`: the triggering
  request spends allowance in the new window.
- `expired_callers_release_live_heap_bytes_on_normal_access`: a dedicated
  single-test binary measures allocator bytes for 20,000 caller identities.
  Peak increase: 3,125,648 bytes. Retained after expiration and the next
  ordinary request: 223 bytes. This measures live allocations, not RSS; the OS
  allocator may retain freed pages. Idle state is reclaimed on the next
  request, with no background worker.

No external dependencies were added. Smoosh is not a separate gate for this
feature because neither `mash` nor `malt-tools` changes; the workspace suite
still includes its normal conformance runner. macOS is unsupported (ADR-0006).

US1 checkpoint: workspace build, exact workspace tests, format, and strict
workspace Clippy all passed on Windows.
