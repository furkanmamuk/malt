# Release Notes

## 2026-10-27 — Gateway hardening

Gateway allowances now recover automatically: 100 admitted requests per caller
per 60 seconds by default, plus a shared ceiling of 1,000. HTTP 429 retains
`code: rate_limited` and adds `cause` (`per_caller` or `system_wide`),
`retry_after_secs`, and a derived `Retry-After` header. Clients can use the
response's delay instead of restarting the daemon or rotating credentials.
Successful and error responses expose caller/global budgets and active limits.

Every route now has an explicit 2 MiB request-body ceiling, retaining axum's
previous JSON default. Oversize replies use structured HTTP 413 with
`code: payload_too_large` and name the limit. Declared oversize is rejected
before reading; other bodies are stopped when they cross the bound.

Operators can set `MALT_GATEWAY_REQUESTS_PER_WINDOW`,
`MALT_GATEWAY_GLOBAL_REQUESTS_PER_WINDOW`, `MALT_GATEWAY_WINDOW_SECS`, and
`MALT_GATEWAY_BODY_LIMIT_BYTES` before daemon startup. Values must be positive
integers. Existing daemons need a restart to adopt the new code and settings.
See `specs/010-gateway-hardening/contracts/limits.md` for header semantics.

## 2026-07-26 — Fail-Closed Session Isolation

Breaking protocol and API change: `SessionInfo.isolation` is now an
`IsolationStatus` object (`effective`, `requested`, `basis`, `mechanism`, and
`detail`) rather than a bare `IsolationTier`. Consumers must read the
effective tier and basis instead of treating a request as proof of containment.

Naming a non-bare tier now defaults to `required`. Calls that historically
received a silently uncontained session can therefore fail with
`isolation_unavailable`. Pass `isolation_policy=preferred` (or
`malt new --isolation-policy preferred`) to explicitly accept a visible
downgrade; use `disabled` to request the bare baseline deliberately.
