# Gateway hardening: implementation and evidence

Date: 2026-10-27. Baseline: `9b3e327` on Windows.

## Survey and correction to the plan

The limiter is reachable from the daemon's final `with_auth` router assembly,
before the scope check. It has no clock and no production reset caller. The
first story replaces the counter with monotonic fixed windows and reclaims
expired keys plus excess hash-table capacity on ordinary request access.

The body-limit premise in the July specification was too broad. Axum's
`Bytes`/`Json` extractors already default to a 2 MiB limit (`axum-core`'s
`extract/default_body_limit.rs` and `ext_traits/request.rs`). Absence of an
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

## US2: body bound

The pre-change declared-length regression returned 413 only after producing
33 chunks of 64 KiB; it failed the zero-polls assertion. With the router
middleware, a declared 128 MiB request is refused with zero source polls.
Unknown and falsely small lengths consume exactly 33 chunks (the 2 MiB bound
plus one crossing chunk), then stop producing the remaining ~126 MiB.
These source-consumption tests establish bounded reading directly; they do
not claim to measure process RSS.

Measured serialized request fixtures: 65,562-byte exec (64 KiB comment plus a
real command), 8,204-byte send (8 KiB raw input), and a valid JSON command
padded to exactly 2,097,152 bytes. All pass through the authenticated routes.
The default retains axum's 2 MiB JSON ceiling and applies it explicitly to
all routes, including routes added before the final `with_auth` call.
The limit counts encoded HTTP body bytes; no decompression layer is installed.

US2 checkpoint: workspace build/test, format, and strict workspace Clippy
passed on Windows.
