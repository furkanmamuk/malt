//! Live-platform isolation probes.
//!
//! These tests deliberately touch the operating system rather than merely
//! constructing isolation configuration. They are the baseline evidence for
//! deciding which mechanisms a session may honestly advertise.

#[cfg(windows)]
#[test]
fn job_objects_and_hcs_entry_points_are_exercised_on_the_host() {
    use malt_platform::isolation::{hcs, job_objects};

    let job = job_objects::create_job_object("malt-isolation-reality", 0, 0)
        .expect("real CreateJobObjectA/SetInformationJobObject calls must succeed");
    assert_eq!(
        job_objects::query_active_processes(&job)
            .expect("real QueryInformationJobObject call must succeed"),
        0
    );

    // The HCS availability probe is a real filesystem/loader prerequisite
    // check. Creating a system is intentionally not attempted when this
    // build lacks the HCS backend: a successful fake or feature-disabled call
    // would not establish host containment.
    if hcs::hcs_available() {
        let error = hcs::create_compute_system(&hcs::HcsConfig {
            id: "malt-isolation-reality".to_string(),
            config_json: "{}".to_string(),
        })
        .expect_err("an HCS system requires the compiled native backend and a valid host config");
        assert!(
            !error.to_string().is_empty(),
            "a host-level HCS refusal must remain diagnosable"
        );
    }
}

/// Dropping the session's last Job Object handle is the teardown operation
/// MALT performs when its worker exits. This observes the child from outside
/// the Job Object and fails if teardown merely releases bookkeeping while the
/// process keeps running.
#[cfg(windows)]
#[test]
fn job_object_teardown_kills_its_real_process_tree() {
    use malt_platform::isolation::job_objects;
    use std::process::Command;
    use std::time::{Duration, Instant};

    let mut child = Command::new("ping")
        .args(["-n", "30", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn a real long-running process");

    {
        let job = job_objects::create_job_object("malt-isolation-teardown", 0, 0)
            .expect("create real Job Object");
        job_objects::assign_process_to_job(&job, child.id())
            .expect("assign child to the session Job Object");
        assert_eq!(
            job_objects::query_active_processes(&job).expect("enumerate live Job Object"),
            1
        );
    }

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if child.try_wait().expect("inspect child state").is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("dropping the Job Object did not terminate its assigned process");
}

/// Helper invoked in a separate test-process by
/// `capped_memory_limit_binds_where_restricted_does_not`. Keeping allocation
/// outside this process makes the observed exit status a property of the Job
/// Object's child, not of the test runner's own address space.
#[cfg(windows)]
#[test]
#[ignore]
fn isolation_memory_hog_helper() {
    let mut bytes = vec![0u8; 64 * 1024 * 1024];
    for byte in bytes.iter_mut().step_by(4096) {
        *byte = 1;
    }
    assert_eq!(bytes[0], 1);
}

/// Restricted supplies process grouping but no resource cap; Capped adds a
/// real 16 MiB per-process limit. The same child work must therefore complete
/// outside the cap and fail inside it. This observes runtime behavior instead
/// of comparing the limit values passed to `SetInformationJobObject`.
#[cfg(windows)]
#[test]
fn capped_memory_limit_binds_where_restricted_does_not() {
    use malt_platform::isolation::{job_objects, IsolationContext};
    use malt_platform::process::{spawn_in_isolation, SpawnConfig};
    use std::sync::Arc;

    fn spawn_memory_hog(context: &IsolationContext) -> malt_platform::process::Child {
        let config =
            SpawnConfig::new(std::env::current_exe().expect("current integration-test executable"))
                .args([
                    "--exact",
                    "isolation_memory_hog_helper",
                    "--ignored",
                    "--nocapture",
                ]);
        spawn_in_isolation(config, context).expect("spawn the isolated memory-hog helper")
    }

    let restricted = IsolationContext::restricted();
    restricted
        .establish_job_object(Arc::new(
            job_objects::create_job_object(
                &format!("malt-isolation-restricted-memory-{}", std::process::id()),
                0,
                0,
            )
            .expect("create restricted job"),
        ))
        .expect("establish restricted job");
    let restricted_status = spawn_memory_hog(&restricted)
        .wait()
        .expect("wait for unrestricted helper");
    assert!(
        restricted_status.success(),
        "the same work must complete when no Capped resource limit is present"
    );

    let capped_job = job_objects::create_job_object(
        &format!("malt-isolation-capped-memory-{}", std::process::id()),
        16,
        0,
    )
    .expect("create capped Job Object");
    let capped = IsolationContext::capped();
    capped
        .establish_job_object(Arc::new(capped_job))
        .expect("establish capped job");
    let mut capped_child = spawn_memory_hog(&capped);
    let capped_status = capped_child.wait().expect("wait for capped helper");
    assert!(
        !capped_status.success(),
        "a 64 MiB allocation must not complete inside the 16 MiB capped Job Object"
    );
}

/// The first action in a freshly launched process checks membership in the
/// specific session job, not just any outer CI/terminal job.
#[cfg(windows)]
#[test]
#[ignore = "subprocess probe invoked by isolated_launch_is_assigned_before_child_code"]
fn isolation_membership_helper() {
    use std::ffi::CString;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{IsProcessInJob, OpenJobObjectA};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let name = CString::new(std::env::var("MALT_TEST_JOB_NAME").expect("child job name")).unwrap();
    // SAFETY: name is NUL-terminated; query-only handle is closed below.
    let job = unsafe {
        OpenJobObjectA(0x0004 /* JOB_OBJECT_QUERY */, 0, name.as_ptr().cast())
    };
    assert!(!job.is_null());
    let mut member = 0;
    // SAFETY: both handles are valid and the output pointer is writable.
    let result = unsafe { IsProcessInJob(GetCurrentProcess(), job, &mut member) };
    // SAFETY: this test owns the handle returned by OpenJobObjectA.
    unsafe {
        CloseHandle(job);
    }
    assert_ne!(result, 0);
    assert_ne!(
        member, 0,
        "child code ran before session Job Object assignment"
    );
}

#[cfg(windows)]
#[test]
fn isolated_launch_is_assigned_before_child_code() {
    use malt_platform::isolation::{job_objects, IsolationContext};
    use malt_platform::process::{spawn_in_isolation, Io, SpawnConfig};
    use std::sync::Arc;
    let name = format!("malt-first-instruction-{}", std::process::id());
    let context = IsolationContext::restricted();
    context
        .establish_job_object(Arc::new(
            job_objects::create_job_object(&name, 0, 0).unwrap(),
        ))
        .unwrap();
    for _ in 0..16 {
        let config = SpawnConfig::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "isolation_membership_helper",
                "--ignored",
                "--nocapture",
            ])
            .env("MALT_TEST_JOB_NAME", &name)
            .stdin(Io::Null)
            .stdout(Io::Null)
            .stderr(Io::Null);
        assert!(spawn_in_isolation(config, &context)
            .unwrap()
            .wait()
            .unwrap()
            .success());
    }
}

#[cfg(windows)]
#[test]
fn assignment_failure_never_executes_child_and_invalidates_session() {
    use malt_platform::isolation::{job_objects, IsolationContext};
    use malt_platform::process::{spawn_in_isolation, Io, SpawnConfig, SpawnError};
    use std::sync::Arc;
    use windows_sys::Win32::System::JobObjects::{
        JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    let job = Arc::new(
        job_objects::create_job_object(
            &format!("malt-admission-failure-{}", std::process::id()),
            0,
            0,
        )
        .unwrap(),
    );
    let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        BasicLimitInformation:
            windows_sys::Win32::System::JobObjects::JOBOBJECT_BASIC_LIMIT_INFORMATION {
                LimitFlags: JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                ActiveProcessLimit: 1,
                ..Default::default()
            },
        ..Default::default()
    };
    // SAFETY: job is live, and limits has the exact extended-information ABI.
    assert_ne!(
        unsafe {
            SetInformationJobObject(
                job.handle() as _,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        },
        0
    );
    let context = IsolationContext::restricted();
    context.establish_job_object(job.clone()).unwrap();
    let holder = SpawnConfig::new("cmd.exe")
        .args(["/d", "/c", "set /p hold="])
        .stdin(Io::Pipe)
        .stdout(Io::Null)
        .stderr(Io::Null);
    let mut holder = spawn_in_isolation(holder, &context).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("escaped.txt");
    let request = || {
        SpawnConfig::new("cmd.exe")
            .args([
                "/d",
                "/c",
                &format!("echo escaped>\"{}\"", marker.display()),
            ])
            .stdin(Io::Null)
            .stdout(Io::Null)
            .stderr(Io::Null)
    };
    assert!(matches!(
        spawn_in_isolation(request(), &context),
        Err(SpawnError::Isolation(_))
    ));
    assert!(context.loss_reason().unwrap().is_some());
    // Even after the capacity is freed, loss is terminal rather than a silent
    // retry with weaker enforcement or an implicit upgrade.
    drop(holder.take_stdin());
    holder.wait().unwrap();
    assert!(matches!(
        spawn_in_isolation(request(), &context),
        Err(SpawnError::Isolation(_))
    ));
    assert!(!marker.exists());
}

#[cfg(windows)]
#[test]
fn restricted_teardown_kills_work_that_bare_teardown_leaves_running() {
    use malt_platform::isolation::{job_objects, IsolationContext};
    use malt_platform::process::{spawn_in_isolation, Io, SpawnConfig};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    for restricted in [false, true] {
        let context = if restricted {
            let context = IsolationContext::restricted();
            context
                .establish_job_object(Arc::new(
                    job_objects::create_job_object(
                        &format!("malt-adjacent-teardown-{}", std::process::id()),
                        0,
                        0,
                    )
                    .unwrap(),
                ))
                .unwrap();
            context
        } else {
            IsolationContext::bare()
        };
        let config = SpawnConfig::new("cmd.exe")
            .args(["/d", "/c", "set /p hold="])
            .stdin(Io::Pipe)
            .stdout(Io::Null)
            .stderr(Io::Null);
        let mut child = spawn_in_isolation(config, &context).unwrap();
        assert!(child.try_wait().unwrap().is_none());
        drop(context);
        let exited = if restricted {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if child.try_wait().unwrap().is_some() {
                    break true;
                }
                if Instant::now() >= deadline {
                    break false;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        } else {
            child.try_wait().unwrap().is_some()
        };
        // Release the input barrier even on a failed assertion, so the test
        // never leaves its own child waiting outside a job.
        drop(child.take_stdin());
        child.wait().unwrap();
        assert_eq!(exited, restricted);
    }
}

#[cfg(windows)]
#[test]
fn capped_cpu_policy_is_read_back_from_the_real_scheduler() {
    use malt_platform::isolation::job_objects;
    use windows_sys::Win32::System::JobObjects::{
        JobObjectCpuRateControlInformation, QueryInformationJobObject,
        JOBOBJECT_CPU_RATE_CONTROL_INFORMATION, JOB_OBJECT_CPU_RATE_CONTROL_ENABLE,
        JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
    };
    let job =
        job_objects::create_job_object(&format!("malt-cpu-policy-{}", std::process::id()), 0, 20)
            .unwrap();
    let mut info = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION::default();
    // SAFETY: the owned job and correctly sized writable output record live
    // throughout the query; no return-length output is needed.
    assert_ne!(
        unsafe {
            QueryInformationJobObject(
                job.handle() as _,
                JobObjectCpuRateControlInformation,
                (&mut info as *mut JOBOBJECT_CPU_RATE_CONTROL_INFORMATION).cast(),
                std::mem::size_of_val(&info) as u32,
                std::ptr::null_mut(),
            )
        },
        0
    );
    assert_eq!(
        info.ControlFlags,
        JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP
    );
    // SAFETY: the ENABLE/HARD_CAP policy selects this member of the union.
    assert!(unsafe { info.Anonymous.CpuRate } > 0);
}

#[cfg(windows)]
#[test]
#[ignore = "subprocess CPU probe invoked by capped_cpu_throttles_real_work"]
fn isolation_cpu_probe_helper() {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    fn cpu_ticks() -> u64 {
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: the current-process pseudo-handle is valid and each time
        // output points to a live writable FILETIME record.
        assert_ne!(
            unsafe {
                GetProcessTimes(
                    GetCurrentProcess(),
                    &mut created,
                    &mut exited,
                    &mut kernel,
                    &mut user,
                )
            },
            0
        );
        let ticks =
            |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
        ticks(kernel) + ticks(user)
    }
    let started = cpu_ticks();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut value = 1u64;
    while std::time::Instant::now() < deadline {
        for _ in 0..4096 {
            value = std::hint::black_box(value.wrapping_mul(1664525).wrapping_add(1013904223));
        }
    }
    let milliseconds = (cpu_ticks() - started) / 10_000;
    std::fs::write(
        std::env::var_os("MALT_TEST_CPU_REPORT").unwrap(),
        milliseconds.to_string(),
    )
    .unwrap();
}

#[cfg(windows)]
#[test]
fn capped_cpu_throttles_real_work() {
    use malt_platform::isolation::{job_objects, IsolationContext};
    use malt_platform::process::{spawn_in_isolation, Io, SpawnConfig};
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let measure = |rate: u32| {
        let report = dir.path().join(format!("cpu-{rate}.txt"));
        let context = IsolationContext::capped();
        context
            .establish_job_object(Arc::new(
                job_objects::create_job_object(
                    &format!("malt-cpu-probe-{}-{rate}", std::process::id()),
                    0,
                    rate,
                )
                .unwrap(),
            ))
            .unwrap();
        let config = SpawnConfig::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "isolation_cpu_probe_helper",
                "--ignored",
                "--nocapture",
            ])
            .env("MALT_TEST_CPU_REPORT", report.as_os_str())
            .stdin(Io::Null)
            .stdout(Io::Null)
            .stderr(Io::Null);
        assert!(spawn_in_isolation(config, &context)
            .unwrap()
            .wait()
            .unwrap()
            .success());
        std::fs::read_to_string(report)
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    let unrestricted = measure(0);
    let capped = measure(20);
    println!(
        "same 2-second CPU workload: unrestricted={unrestricted}ms CPU, capped={capped}ms CPU"
    );
    // A 20%-of-one-core hard cap allows ~400ms CPU in this window. Allow one
    // scheduling interval of burst/timer granularity, not unlimited use.
    assert!(capped < 900, "CPU hard cap did not bind: {capped}ms");
    if unrestricted < 1000 {
        eprintln!("SKIP adjacent CPU ratio proof: competing host load left only {unrestricted}ms baseline CPU; hard-cap budget assertion and scheduler read-back still ran");
        return;
    }
    assert!(
        capped * 2 < unrestricted,
        "same work was not meaningfully throttled"
    );
}

#[cfg(not(windows))]
#[test]
fn windows_isolation_paths_are_not_claimed_on_other_platforms() {
    // This is a live-build assertion, not a constructed capability report:
    // the Windows modules are cfg-gated out, so this target cannot honestly
    // establish a Windows session tier.
    assert!(!cfg!(windows));
}

/// Probe: can this host actually create an HCS compute system?
///
/// **The assertion is that this test returns at all.** It used to terminate
/// the process with `STATUS_ACCESS_VIOLATION`, because
/// `HcsStartComputeSystem` was called with a null `HCS_OPERATION` handle and
/// computecore dereferences it unconditionally. No configuration can make a
/// crash acceptable: a request for an unavailable tier must produce an error a
/// caller can read, and a faulting daemon takes every other session with it.
/// So this runs unconditionally rather than `#[ignore]`d — an ignored test
/// cannot catch that regression, and for months it did not.
///
/// It deliberately does *not* assert `Ok`. Whether a compute system can be
/// created is genuinely host-dependent: it needs the Windows Containers
/// feature, Hyper-V Administrators rights, and a base image layer. The
/// outcome is printed so a reader can see which of those this host lacks.
#[cfg(windows)]
#[test]
fn hcs_create_never_faults_whatever_this_host_supports() {
    use malt_platform::isolation::hcs;

    println!("hcs_available() = {}", hcs::hcs_available());
    match hcs::ensure_hcs_runtime() {
        Ok(()) => println!("ensure_hcs_runtime() = Ok"),
        Err(e) => println!("ensure_hcs_runtime() = Err: {e}"),
    }

    // A structurally valid minimal Windows container document. Creating one
    // still needs a base layer, so a failure here distinguishes "feature
    // absent" from "feature present, image missing".
    let config = hcs::HcsConfig {
        id: format!("malt-hcs-probe-{}", std::process::id()),
        config_json: r#"{"SchemaVersion":{"Major":2,"Minor":1},"Owner":"malt","ShouldTerminateOnLastHandleClosed":true}"#
            .to_string(),
    };
    match hcs::create_compute_system(&config) {
        Ok(system) => {
            println!(
                "create_compute_system() = Ok (handle {})",
                system.raw_handle()
            );
            // Tearing down exercises the other call site that passed a null
            // operation handle, so it faulted for the same reason. A host that
            // can create a compute system must also be able to destroy one.
            let handle = system.raw_handle();
            std::mem::forget(system);
            hcs::terminate_compute_system(handle)
                .expect("a compute system that could be created must be terminable");
        }
        Err(e) => {
            let message = e.to_string();
            println!("create_compute_system() = Err: {message}");
            // A bare `HRESULT=0x8037011b` is what kept the real cause -- the
            // daemon not holding Hyper-V Administrators rights -- unreadable
            // once the crash was fixed. Every HCS HRESULT this module reports
            // goes through the decoder, so a raw code with no name means a
            // call site bypassed it.
            if message.contains("HRESULT=0x8037") {
                assert!(
                    message.contains("HCS_E_"),
                    "HCS errors must name the failure, not just its hex code: {message}"
                );
            }
        }
    }
}

/// The HCS capability report must not claim more than it checked.
///
/// `computecore.dll` ships with Windows regardless of the Containers feature
/// or of whether this build compiled the HCS backend in. Reporting `Verified`
/// on its presence advertised `Contained` as available on hosts where every
/// required request for it then failed -- a capabilities answer that
/// disagrees with reality, which is worse than none because it is what a
/// caller uses to decide what to ask for (SC-007).
///
/// This asserts the report is never `Verified`, on either build. Without the
/// `hcs` feature the runtime check fails and it must be `Unsupported`; with
/// it, the prerequisites resolve but no compute system has been created, so
/// the honest basis is `Assumed`.
#[cfg(windows)]
#[test]
fn hcs_capability_is_never_reported_as_verified_on_dll_presence_alone() {
    use malt_platform::isolation::probe::IsolationCapabilities;
    use malt_platform::isolation::{CapabilityBasis, CapabilityStatus};

    let caps = IsolationCapabilities::probe();
    let hcs = &caps.windows_hcs;

    assert_ne!(
        hcs.basis,
        CapabilityBasis::Verified,
        "HCS was reported as Verified, but nothing created a compute system;          the only check performed is that computecore.dll exists, which is          true on stock Windows. Report: {hcs:?}"
    );

    if hcs.status == CapabilityStatus::Supported {
        assert_eq!(
            hcs.basis,
            CapabilityBasis::Assumed,
            "a supported-but-unverified capability must say Assumed: {hcs:?}"
        );
    }
}

/// Print this host's full capability probe. Runs on **every** platform.
///
/// Added after the CI `isolation-capabilities` job was reported as passing on
/// Linux and macOS when it had in fact run a single negative test — every
/// other test in this file is `#[cfg(windows)]`, so the job was green on
/// hosts where isolation is entirely unwired. A green tick that would stay
/// green if the feature were absent is the exact failure this repo keeps
/// producing.
///
/// This asserts nothing about facets it cannot reason about; its job is to
/// make the answer visible in the CI log, which is what was claimed and not
/// delivered. The assertions that *can* hold everywhere are in the test
/// below.
#[test]
fn print_this_hosts_isolation_capabilities() {
    use malt_platform::isolation::IsolationCapabilities;

    let caps = IsolationCapabilities::probe();
    println!("--- isolation capabilities on {} ---", std::env::consts::OS);
    for (name, report) in [
        ("linux_namespaces", &caps.linux_namespaces),
        ("linux_cgroups", &caps.linux_cgroups),
        ("linux_overlayfs", &caps.linux_overlayfs),
        ("linux_seccomp", &caps.linux_seccomp),
        ("windows_job_objects", &caps.windows_job_objects),
        ("windows_restricted_tokens", &caps.windows_restricted_tokens),
        ("windows_hcs", &caps.windows_hcs),
        ("macos_sandbox", &caps.macos_sandbox),
        ("macos_rlimit", &caps.macos_rlimit),
    ] {
        println!(
            "  {name:<26} {:?} / {:?} ({:?}){}",
            report.status,
            report.basis,
            report.reason_code,
            report
                .reason_detail
                .as_deref()
                .map(|d| format!(" — {d}"))
                .unwrap_or_default()
        );
    }
    println!(
        "  tiers: restricted={} capped={} contained={}",
        caps.supports_restricted(),
        caps.supports_capped(),
        caps.supports_contained()
    );
}

/// The two things the probe says about a tier must agree, on every platform.
///
/// `supports_<tier>()` answers "is this available here" and
/// `unsupported_detail_for_tier()` answers "why not". A tier reported as
/// available while also carrying an unsupported reason is two surfaces
/// disagreeing about one fact — FR-007 — and it is a property that holds
/// regardless of which mechanisms a host actually has, so it is checkable on
/// Linux and macOS today rather than after those paths are wired.
#[test]
fn tier_availability_and_unsupported_detail_never_disagree() {
    use malt_platform::isolation::{IsolationCapabilities, IsolationTier};

    let caps = IsolationCapabilities::probe();
    for tier in [
        IsolationTier::Bare,
        IsolationTier::Restricted,
        IsolationTier::Capped,
        IsolationTier::Contained,
    ] {
        let available = match tier {
            IsolationTier::Bare => true,
            IsolationTier::Restricted => caps.supports_restricted(),
            IsolationTier::Capped => caps.supports_capped(),
            IsolationTier::Contained => caps.supports_contained(),
        };
        let detail = caps.unsupported_detail_for_tier(tier);
        assert_eq!(
            available,
            detail.is_none(),
            "on {}: {tier:?} reports available={available} but unsupported_detail={detail:?}; \
             the two surfaces must not disagree about the same tier",
            std::env::consts::OS
        );
    }
}
