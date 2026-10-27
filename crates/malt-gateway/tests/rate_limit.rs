use malt_gateway::rate_limit::RateLimiter;

#[test]
fn under_limit_allowed() {
    let limiter = RateLimiter::new(10);
    for _ in 0..10 {
        assert!(limiter.check("client-1"));
    }
}

#[test]
fn at_limit_rejected() {
    let limiter = RateLimiter::new(5);
    for _ in 0..5 {
        assert!(limiter.check("client-1"));
    }
    assert!(!limiter.check("client-1"));
}

#[test]
fn per_client_isolation() {
    let limiter = RateLimiter::new(2);
    assert!(limiter.check("client-1"));
    assert!(limiter.check("client-1"));
    assert!(!limiter.check("client-1"));
    // client-2 is unaffected
    assert!(limiter.check("client-2"));
    assert!(limiter.check("client-2"));
    assert!(!limiter.check("client-2"));
}

#[test]
fn elapsed_window_restores_allowance_without_a_reset_call() {
    let window = std::time::Duration::from_millis(20);
    let limiter = RateLimiter::with_window(2, window).unwrap();
    assert!(limiter.check("client-1"));
    assert!(limiter.check("client-1"));
    assert!(!limiter.check("client-1"));
    std::thread::sleep(window);
    assert!(limiter.check("client-1"));
    assert!(limiter.check("client-1"));
    assert!(!limiter.check("client-1"));
}

#[tokio::test]
async fn authenticated_caller_recovers_and_another_token_is_unaffected() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
        Router,
    };
    use malt_gateway::{
        auth::{AuthScope, TokenStore},
        with_auth,
    };
    use std::{sync::Arc, time::Duration};
    use tower::ServiceExt;

    let store = Arc::new(TokenStore::new());
    let a = store.generate_token(AuthScope::Monitor).unwrap();
    let b = store.generate_token(AuthScope::Monitor).unwrap();
    let window = Duration::from_millis(80);
    let limiter = Arc::new(RateLimiter::with_window(2, window).unwrap());
    let router = with_auth(
        Router::new().route("/health", get(|| async { "ok" })),
        store,
        limiter,
    );
    let request = |token: &str| {
        Request::builder()
            .uri("/health")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        router.clone().oneshot(request(&a)).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        router.clone().oneshot(request(&a)).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        router.clone().oneshot(request(&a)).await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        router.clone().oneshot(request(&b)).await.unwrap().status(),
        StatusCode::OK
    );
    // Sleeping establishes actual passage of time here, not a scheduling
    // precondition for another worker.
    for _ in 0..4 {
        std::thread::sleep(window);
        assert_eq!(
            router.clone().oneshot(request(&a)).await.unwrap().status(),
            StatusCode::OK
        );
        assert_eq!(
            router.clone().oneshot(request(&a)).await.unwrap().status(),
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn refusal_retry_headers_drive_a_successful_retry_and_identify_the_cause() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
        Router,
    };
    use malt_gateway::{
        auth::{AuthScope, TokenStore},
        limits::GatewayLimits,
        with_auth_configured,
    };
    use std::{sync::Arc, time::Duration};
    use tower::ServiceExt;
    for system_refusal in [false, true] {
        let store = Arc::new(TokenStore::new());
        let a = store.generate_token(AuthScope::Monitor).unwrap();
        let b = store.generate_token(AuthScope::Monitor).unwrap();
        let limits = GatewayLimits {
            per_caller: if system_refusal { 2 } else { 1 },
            system_wide: if system_refusal { 1 } else { 10 },
            window: Duration::from_millis(40),
            ..GatewayLimits::default()
        };
        let router = with_auth_configured(
            Router::new().route("/health", get(|| async { "ok" })),
            store,
            limits,
        )
        .unwrap();
        let request = |token: &str| {
            Request::builder()
                .uri("/health")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap()
        };
        let ok = router.clone().oneshot(request(&a)).await.unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(
            ok.headers()["x-ratelimit-remaining"],
            if system_refusal { "1" } else { "0" }
        );
        let refused_token = if system_refusal { &b } else { &a };
        let refused = router
            .clone()
            .oneshot(request(refused_token))
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
        let delay: u64 = refused.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            delay, 1,
            "round the real sub-second remainder up, never down"
        );
        let bytes = axum::body::to_bytes(refused.into_body(), 4096)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            json["error"]["cause"],
            if system_refusal {
                "system_wide"
            } else {
                "per_caller"
            }
        );
        assert_eq!(json["error"]["retry_after_secs"], delay);
        std::thread::sleep(Duration::from_secs(delay));
        assert_eq!(
            router
                .oneshot(request(refused_token))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
}

#[test]
fn concurrent_admission_never_overspends_either_budget() {
    use std::sync::{Arc, Barrier};
    use std::time::Duration;
    let limiter = Arc::new(RateLimiter::with_limits(5, 9, Duration::from_secs(60)).unwrap());
    let barrier = Arc::new(Barrier::new(24));
    let threads: Vec<_> = (0..24)
        .map(|i| {
            let limiter = limiter.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                (i % 2, limiter.check(if i % 2 == 0 { "a" } else { "b" }))
            })
        })
        .collect();
    let mut admitted = [0; 2];
    for thread in threads {
        let (id, allowed) = thread.join().unwrap();
        admitted[id] += usize::from(allowed);
    }
    assert_eq!(admitted.iter().sum::<usize>(), 9);
    assert!(admitted.iter().all(|used| *used <= 5));
}
