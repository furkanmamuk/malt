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

## US3: actionable refusals and aggregate admission

The same mutex guards global and per-caller admission. Both are checked
before spending either budget; denied rate-limit requests spend neither.
Admitted requests still spend quota before the scope check. Refusals name
`per_caller` or `system_wide`, preserve HTTP 429, and derive integral
`Retry-After` seconds by rounding the real monotonic remainder upward. If
both budgets are exhausted, the wait covers both expirations.

Runtime metadata on every response includes per-caller limit, remaining and
reset, global limit, remaining and reset, window seconds, and body bytes.
Remaining values are snapshots at admission; concurrent requests may consume
additional allowance afterward. Unauthenticated replies report zero available
caller allowance and do not consume the authenticated aggregate budget.

Defaults: 100 requests/caller per 60 seconds, 1,000 requests globally per the
same window (ten full caller allowances), and 2,097,152 body bytes. Overrides
are `MALT_GATEWAY_REQUESTS_PER_WINDOW`,
`MALT_GATEWAY_GLOBAL_REQUESTS_PER_WINDOW`, `MALT_GATEWAY_WINDOW_SECS`, and
`MALT_GATEWAY_BODY_LIMIT_BYTES`. All must be positive integers; malformed or
zero values fail daemon startup. No new dependency was needed. Wiring the
settings in `malt-bin/src/daemon.rs` was necessary to make configuration
reachable, extending the plan's initially gateway-only file list.

Focused proof includes a 24-thread admission race (nine total admissions,
no caller over five), successful retry using only the refusal's delay,
decreasing budgets on success and scope refusal, configuration validation,
and coverage of routes added after `build_router`.

## Live quickstart evidence

`live_http_clients_recover_and_distinguish_shared_exhaustion` starts the real
HTTP router on a random loopback port with three distinct valid tokens. TCP
clients prove caller exhaustion/recovery, another token's immediate service,
three sustained windows, response-only retry, global exhaustion with a
`system_wide` cause, and declining remaining headers. A declared 128 MiB
request receives 413 even though the client sends no body. Its backend is a
session fixture; it does not claim MASH execution.

`pwsh -NoProfile -File scripts/verify-gateway-hardening.ps1` separately starts
the actual `malt daemon` executable on an isolated port and session store.
Observed on Windows:

- allowance headers: `2,1,0`, followed by 429 with `per_caller` cause;
- returned retry delay: 2 seconds; exactly that wait restored 200;
- valid 2,097,152-byte JSON request executed MASH and returned
  `gateway-live-ok`;
- meaningful serialized exec fixture: 65,572 bytes; send fixture: 8,204 bytes;
- a 134,217,728-byte content stream with Expect/Continue was refused with
  `payload_too_large`; sampled child working-set increase: 0 bytes;
- session destruction succeeded, and graceful daemon shutdown exited 0.

This RSS measurement applies to declared-length early rejection. For unknown
or understated lengths, source-poll measurements prove that reading stops at
the first crossing chunk; they do not measure RSS. SC-008 uses the dedicated
allocator measurement above, since OS allocators need not return freed pages
to RSS. The TCP fixture supplies multiple credentials without adding a
credential-issuance API to the CLI daemon.

US3 checkpoint: workspace build/test, format, and strict workspace Clippy
passed on Windows. Linux mirror verification follows the checkpoint commit.

## Final gates and cleanup

Final Windows workspace build/test, format, and strict workspace Clippy passed.
The repeat CLI probe again returned the same values above and exited 0; its
temporary 128 MiB payload was removed. The helper remained installed/stopped,
and no developer session was created or destroyed by the proof fixture.

`bash scripts/wsl-mirror.sh` first stopped before compilation because the
Windows checkout had CRLF shell-script endings. A repository `*.sh text eol=lf`
attribute and normalization fixed that entry point. The mirror then synced
commit `771cb96`, built the complete workspace on the Linux filesystem, and
passed the complete workspace tests, including conformance and doc-tests.
Existing Linux-only unused-import/variable warnings in platform/helper-client
code remained warnings; Linux strict Clippy was not claimed as a gate.

Spec 008's stale Draft status now points to its completed task list and prior
host evidence. Spec 007 records its delivered Windows helper/image prerequisite
and explicitly excludes macOS wiring under ADR-0006, while retaining its
unfinished comparisons, Linux backend, containment-loss, and quickstart work.
The rate-limit brief, backlog, roadmap, implemented inventory, and release
notes now reflect Spec 010. No new hosted CI run is claimed by these local
checks.
