use std::sync::atomic::{AtomicBool, Ordering};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use std::thread;
use tokio::runtime::Builder as RtBuilder;

static SERVER_RUNNING: AtomicBool = AtomicBool::new(false);

pub(crate) fn setup_profiler_server_sync() {
    if SERVER_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    thread::spawn(move || {
        let rt = RtBuilder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            setup_profiler_server().await;
        });
    });
}

async fn setup_profiler_server() {
    tokio::spawn(async move {
        let app = axum::Router::new()
            .route("/debug/pprof/allocs", axum::routing::get(handle_get_heap))
            .route("/debug/pprof/allocs/flamegraph", axum::routing::get(handle_get_heap_flamegraph));

        let listener = tokio::net::TcpListener::bind("0.0.0.0:7421").await.unwrap();
        axum::serve(listener, app).await.unwrap();
    });
}


pub async fn handle_get_heap() -> Result<impl IntoResponse, (StatusCode, String)> {
    let mut prof_ctl = jemalloc_pprof::PROF_CTL.as_ref().unwrap().lock().await;
    require_profiling_activated(&prof_ctl)?;
    let pprof = prof_ctl
        .dump_pprof()
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(pprof)
}

/// Checks whether jemalloc profiling is activated an returns an error response if not.
fn require_profiling_activated(prof_ctl: &jemalloc_pprof::JemallocProfCtl) -> Result<(), (StatusCode, String)> {
    if prof_ctl.activated() {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, "heap profiling not activated".into()))
    }
}

pub async fn handle_get_heap_flamegraph() -> Result<impl IntoResponse, (StatusCode, String)> {
    use axum::body::Body;
    use axum::http::header::CONTENT_TYPE;
    use axum::response::Response;

    let mut prof_ctl = jemalloc_pprof::PROF_CTL.as_ref().unwrap().lock().await;
    require_profiling_activated(&prof_ctl)?;
    let svg = prof_ctl
        .dump_flamegraph()
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Response::builder()
        .header(CONTENT_TYPE, "image/svg+xml")
        .body(Body::from(svg))
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
}