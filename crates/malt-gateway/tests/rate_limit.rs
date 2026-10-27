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
