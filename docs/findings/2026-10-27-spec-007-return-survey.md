# Returning to Spec 007: remaining enforcement work

Date: 2026-10-27. Baseline: pushed `095bb56`.

## Production paths, not definitions

- `mash::Env::spawn_external_process` is the common launcher used by five
  executor paths. It supports an injected launcher and defaults to the
  ordinary platform spawn. Windows HCS injects that launcher in
  `session_thread::apply_session_isolation`.
- Windows Restricted/Capped still rely on
  `mash::executor::assign_child_to_session_job` after the child starts. The
  function explicitly warns and continues when assignment fails. A created
  Job Object is not sufficient proof that its subsequent commands entered it.
- `IsolationContext::clear_established` exists and is shared with the
  coordinator's status view, but no production code calls it when helper
  launch/transport fails. Merely having a shared carrier has not completed
  loss detection (T037).
- Restored Compat panes are launched by `ProcessSupervisor::spawn` through
  `spawn_with_pty`, without the executor's isolation carrier. The source
  already names this gap. This remains relevant to FR-012 and restore proof.

## Linux survey

- No Linux session backend reaches `apply_session_isolation`; every non-Bare
  request is currently refused on Linux. Existing namespace/cgroup/seccomp
  functions are not a session launcher.
- `namespaces::create_pid_namespace` calls `unshare(CLONE_NEWPID)`. That
  changes the namespace for future children, not the caller. Calling it in a
  process's pre-exec callback does not put that process inside a PID namespace.
  Calling these helpers on the multithreaded daemon's session worker is not a
  safe substitute for child-side setup.
- `rlimit.rs` is a macOS implementation despite its generic name. It refuses
  Linux; it is not a ready Linux resource-capping path.
- `seccomp.rs`'s x86-64 allowlist contains mislabeled syscall numbers, and its
  install routine hardcodes the x86-64 syscall ABI and builds the filter at
  call time. It cannot be wired blindly into a post-fork child of a threaded
  daemon. Contained's promises also require filesystem/network configuration.
- WSL's normal user successfully ran a separate user/mount/PID/UTS/IPC
  namespace with util-linux `unshare --fork --mount-proc`; the child saw UID 0
  in its user namespace. The normal user cannot write the root cgroup
  hierarchy; WSL root can, and CPU/memory/pids controllers are enabled.
  This distinguishes a usable host primitive from an existing MALT backend.
- The legacy donor is now `C:/Users/mamuk/projects/LEGACY-vexil-v2`.
  Its `vexil-platform/src/isolation.rs` stores unshare flags for pre-exec;
  that alone does not establish the required PID-namespace child lifecycle.
  No donor code has been copied.

## What remains unverified

This survey does not establish Linux session enforcement, Windows adjacent
tier differences for all pairs, live HCS loss reporting, or complete restore
containment. The corresponding tasks remain open until their actual paths
are exercised. Existing completed Windows image evidence is historical and
does not make a newly changed spawn path verified.

## Windows continuation checkpoint

The common MASH launcher now uses `CREATE_SUSPENDED`, assigns the owned
process handle to the session Job Object, and resumes only after assignment.
Assignment/resume failures terminate the suspended process and return an
isolation error; the old post-spawn best-effort assignment calls are removed.
Native stdio handles are RAII-owned on setup/error paths and inherited
streams are explicitly duplicated rather than passed as invalid handles.

The live carrier has a terminal lost state. It retains Job/container ownership
for cleanup but cannot launch or re-establish in place. Detection is on the
next external-process launch, **not idle query polling**. Native admission
failure, helper unavailability, lost entitlement, malformed response or an
indeterminate HCS launch invalidate that carrier. An ordinary helper OS
refusal (for example a missing executable) does not by itself imply loss.
Coordinator reports read the shared carrier and do not hide loss with old
verification detail. Persistence retains the originally granted requirement,
so a transient loss cannot silently turn into a Bare grant on restart.

Isolated Compat restore is explicitly refused before executor/helper
allocation. The supervisor also rejects non-Bare requests without a
session-owned launcher. This closes the uncontained launch, **not** the
missing isolated Compat capability. Bare Compat restore remains supported.

Observed tests:

- Sixteen fresh child executions checked membership in the specific session
  job as their first test action, not membership in an outer CI job.
- A real one-process job admitted a blocked child, then refused a second
  launch. Its payload marker stayed absent; subsequent launches remained
  refused after capacity was released.
- Dropping a Restricted context killed blocked work; dropping Bare left the
  same work running until the test released stdin.
- Identical 64 MiB allocation completed under Restricted and failed under a
  16 MiB Capped job. The allocation-failure message is the expected child
  outcome, not a workspace test-runner crash. No startup sleep establishes
  assignment anymore.
- A real Restricted coordinator session lost its carrier, reported no
  verified mechanism, refused subsequent external work and persisted the
  Restricted restoration requirement. Persisted isolated Compat restore
  remained dormant and refused before establishment.
- Rust 1.99 Windows workspace build, full tests, format check and strict
  all-targets Clippy passed. Privileged boundary/real-image ignored tests
  were **not** run: the installed helper is stopped and the image records
  directory is empty. No service or image state was changed for this check.

The separate CI repairs reached green on main `413acb9` in
[run 40517994364](https://github.com/furkanmamuk/malt/actions/runs/40517994364).
That run predates these Spec 007 changes. Its Smoosh job was path-skipped;
the earlier repair run `40516456513` actually exercised Smoosh because it
touched MASH. Local continuation Smoosh independently reported 183 passed,
three unsupported skips, zero harness failures and zero shell failures.
