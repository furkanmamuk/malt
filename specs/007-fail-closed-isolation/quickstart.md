# Quickstart: Fail-Closed Session Isolation

**Feature**: `specs/007-fail-closed-isolation/`

**Every scenario must observe a constraint, not a return code.** That is the
specific trap for this feature: the Job Object call *succeeds* today, and
nothing checks that anything is actually constrained by it. A scenario that
confirms creation returned success proves nothing that is not already true
while the defect is fully present.

## Prerequisites

```bash
cargo build --workspace
```

```bash
./target/debug/malt daemon --port 7980
```

---

## Scenario 1 (US1) — a required request that cannot be met fails

Request a tier this host cannot provide. Confirm which that is with Scenario 7
first; on a host without Windows Containers it is `contained`.

```bash
malt new --isolation contained --isolation-policy required
```

**Expected**: a failure naming what could not be provided, and mentioning
`preferred` as the way to accept less.

```bash
malt list
```

**Expected**: **no session was created.** This is the assertion that matters —
"the command printed an error" is entirely compatible with a session still
running, which is the shape of the current defect.

---

## Scenario 2 (US1) — preferred degrades visibly

```bash
malt new --isolation contained --isolation-policy preferred
```

**Expected**: a session is created, and the output states *in that same
response* that it did not get `contained` and what it got instead. No second
command should be needed to discover the downgrade.

---

## Scenario 3 (US2) — every surface agrees

For the session from Scenario 2:

```bash
malt list
```

```bash
curl -s -H "Authorization: Bearer $(cat ~/.config/malt/api-token)" http://127.0.0.1:7980/sessions/1
```

**Expected**: the creation response, the list entry, and the single-session
query report **identical** isolation status, including `basis`. Check all
three for a granted, a downgraded, and (via Scenario 1) a refused request.

---

## Scenario 4 (US3) — tiers actually differ

Run the same work under two adjacent tiers and observe a constraint binding
at the stronger one and not the weaker. For a memory cap, allocate more than
the cap in each:

```bash
malt exec <capped-session-id> 'python3 -c "b = bytearray(3*1024*1024*1024); print(len(b))"'
```

```bash
malt exec <restricted-session-id> 'python3 -c "b = bytearray(3*1024*1024*1024); print(len(b))"'
```

**Expected**: constrained under the stronger tier, unconstrained under the
weaker. **Expected NOT**: both behaving identically — which is exactly
today's behaviour for `capped` versus `contained`.

Repeat for each adjacent pair (SC-004). A pair with no observable difference
is a tier that does not exist.

The current default cap is 2048 MiB, so the old 600 MiB example could not
bind. The 3 GiB example requires enough spare host memory for the Restricted
control. Prefer the automated `capped_memory_limit_binds_where_restricted_does_not`
probe for a smaller fixture: it compares the same 64 MiB work against a
16 MiB limit. A subprocess allocation failure is expected only in the capped
arm; a test-runner crash is never an acceptable result.

---

## Scenario 5 (US1/US3) — a spawned process is inside the containment

```bash
malt exec <contained-session-id> 'python3 -c "import subprocess,time; subprocess.Popen([\"sleep\",\"30\"]); time.sleep(1)"'
```

**Expected**: the child is subject to the session's containment, demonstrated
by a constraint applying to it — not by the parent's creation having
succeeded (FR-012, SC-005).

---

## Scenario 6 — teardown leaves nothing

```bash
malt kill <ID>
```

**Expected**: no process or resource the session held remains, **verified by
inspection** — enumerate the job or container and its processes and find them
gone. SC-006 says this explicitly: not by the absence of an error.

---

## Scenario 7 (US4) — capabilities are discoverable first

```bash
malt isolation capabilities
```

**Expected**: per tier, whether it is available here, by which mechanism, and
on what basis. `assumed` must appear where the probe does not actually check
the host (research R2) rather than being reported as `verified`.

**Cross-check**: what this reports must match which requests then succeed. A
capabilities call that disagrees with reality is worse than none, because it
is precisely what a caller uses to decide what to ask for.

---

## Scenario 8 (FR-014) — restore does not inherit a claim

Create a contained session, stop the daemon, restart it, then:

```bash
malt list
```

**Expected**: the restored session reports containment it **actually holds**.
If it could not be re-established, it says so. It must not report its saved
tier on the strength of the saved record alone.

---

## Gate check before completion

```bash
cargo test --workspace
```

```bash
cargo fmt --all -- --check
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Smoosh **is** a gate for the continuation: the common MASH launcher changes.
Build `mash`, set `MASH` to that executable, then run its conformance runner.
Require 183 passed/three unsupported skips on native Windows; the Linux
mirror exercises its supported Linux cases separately.

## Linux bootstrap continuation

Implementation is pending; this is the validation contract, not a claim that
the following tiers already work. Configure only an explicitly delegated
cgroup subtree and, for Contained, a Linux rootfs through the daemon config
fields introduced by T048. Rootfs packaging/download is not part of Spec 007.
Never run the daemon as root just to turn a Required refusal into success.

Run Scenarios 1–8 against the real built Linux `malt` entry point, not a
threaded test harness standing in for the bootstrap. Also inspect:

- Different child PID/mount namespace identities and proc view, plus
  `NoNewPrivs: 1`, before executing the payload's first action.
- Child/descendant cgroup membership and configured memory/CPU limits;
  compare the same real work under Restricted and Capped.
- A known host-only file and host network endpoint accessible under Capped
  but unavailable under configured Contained; inspect its private root mounts.
- Daemon death, broker death, invalid/partial setup and restore: no escaped
  payload, stale verified status, owned processes, cgroups, mounts or workspace
  remain after confirmed teardown.

On a host without delegation/rootfs/kernel support, record the exact refusal
and visible downgrade. That proves fail-closed behavior, not the missing
positive containment gate. Do not mark T032 complete on skipped positives.

The gate that replaces it: **isolation tests must call real OS APIs.** 54 of
the existing isolation tests pass in 0.01 s, which is what pure-logic tests
that never touch the OS look like. Two real `job_objects.rs` bugs survived
exactly that shape of test. Any tier this feature claims to enforce needs a
test that observes the constraint from outside.
