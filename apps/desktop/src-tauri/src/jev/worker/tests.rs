use super::*;
use crate::jev::config::{ContextLimitOverride, SystemOneEndpoint};
use std::sync::{Arc, Condvar};
use std::time::Duration;

fn active_handle() -> (WorkerHandle, u64) {
    let handle = WorkerHandle::default();
    handle
        .set_system_one_connection(
            SystemOneConnection::jev_default(),
            Some("synthetic-key".into()),
        )
        .unwrap();
    let (_, generation) = handle.execution_client().unwrap();
    (handle, generation)
}

#[test]
fn authentication_rejection_cannot_disable_a_replacement_connection() {
    let store = Store::open_in_memory(std::path::Path::new("synthetic-auth-state")).unwrap();
    let (handle, generation) = active_handle();
    handle
        .set_system_one_connection(
            SystemOneConnection::jev_default(),
            Some("replacement".into()),
        )
        .unwrap();
    assert!(!handle.reject_authentication(&store, generation).unwrap());
    assert!(handle.is_available());
    assert_eq!(
        store.internal_value("internal:typesafeAuthRejectedV1"),
        None
    );
}

#[test]
fn authentication_rejection_preserves_current_generation_settlement() {
    let store = Store::open_in_memory(std::path::Path::new("synthetic-auth-state")).unwrap();
    let (handle, generation) = active_handle();
    assert!(handle.reject_authentication(&store, generation).unwrap());
    assert!(!handle.key_is_current(generation));
    handle
        .with_current_generation(generation, || {
            store.set_internal_value_checked("test:failure-settled", "true")
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        store.internal_value("test:failure-settled").as_deref(),
        Some("true")
    );
    assert_eq!(
        store
            .internal_value("internal:typesafeAuthRejectedV1")
            .as_deref(),
        Some("true")
    );
    assert!(handle.execution_client().is_none());
}

#[test]
fn connection_and_credential_changes_advance_generation_and_fence_old_work() {
    let handle = WorkerHandle::default();
    let initial = handle.key_generation.load(Ordering::Acquire);
    let connection = SystemOneConnection {
        provider: SystemOneProvider::Custom,
        endpoint: SystemOneEndpoint::ExactUrl("https://proxy.example/v1/systemone".into()),
        model: "proxy-model".into(),
        credential: None,
        revision: 2,
        context_override: Some(ContextLimitOverride {
            total_input_tokens: Some(8192),
            ..Default::default()
        }),
        ..SystemOneConnection::default()
    };
    handle
        .set_system_one_connection(connection.clone(), Some("secret-one".into()))
        .unwrap();
    let selected = handle.key_generation.load(Ordering::Acquire);
    assert_eq!(selected, initial + 1);
    assert_eq!(handle.system_one_connection(), connection);
    handle
        .set_system_one_connection(connection.clone(), Some("secret-one".into()))
        .unwrap();
    assert_eq!(handle.key_generation.load(Ordering::Acquire), selected);
    let mut revised = connection;
    revised.revision += 1;
    handle
        .set_system_one_connection(revised, Some("secret-two".into()))
        .unwrap();
    let current = handle.key_generation.load(Ordering::Acquire);
    assert_eq!(current, selected + 1);
    assert!(handle.with_current_generation(selected, || ()).is_none());
    assert!(handle.with_current_generation(current, || ()).is_some());
}

#[test]
fn keyless_provider_connections_are_available_and_current() {
    let handle = WorkerHandle::default();
    let connection = SystemOneConnection {
        provider: SystemOneProvider::Ollama,
        endpoint: SystemOneEndpoint::BaseUrl("http://127.0.0.1:11434".into()),
        model: "clef-flash".into(),
        credential: None,
        revision: 2,
        ..SystemOneConnection::default()
    };
    handle.set_system_one_connection(connection, None).unwrap();
    let (_, generation) = handle.execution_client().unwrap();
    assert!(handle.is_available());
    assert!(handle.key_is_current(generation));
}

#[test]
fn suspending_keyless_providers_fences_dispatch_and_publication_until_explicit_resume() {
    for provider in [SystemOneProvider::Ollama, SystemOneProvider::Custom] {
        let handle = WorkerHandle::default();
        let connection = SystemOneConnection {
            provider,
            endpoint: match provider {
                SystemOneProvider::Ollama => {
                    SystemOneEndpoint::BaseUrl("http://127.0.0.1:11434".into())
                }
                _ => SystemOneEndpoint::ExactUrl("https://proxy.example/systemone".into()),
            },
            model: "clef-flash".into(),
            credential: None,
            context_override: Some(ContextLimitOverride {
                total_input_tokens: Some(8192),
                ..Default::default()
            }),
            ..SystemOneConnection::default()
        };
        handle
            .set_system_one_connection(connection.clone(), None)
            .unwrap();
        let (_, generation) = handle.execution_client().unwrap();
        handle.suspend_system_one();
        let paused = handle.key_generation.load(Ordering::Acquire);
        assert!(!handle.is_available());
        assert!(handle.execution_client().is_none());
        assert!(!handle.key_is_current(generation));
        assert!(!handle.key_is_current(paused));
        assert!(
            handle
                .with_current_generation(generation, || panic!("stale publication"))
                .is_none()
        );
        assert!(
            handle
                .with_current_generation(paused, || panic!("paused publication"))
                .is_none()
        );
        handle
            .install_system_one_connection(connection.clone(), None, false)
            .unwrap();
        assert!(!handle.is_available());
        handle.set_system_one_connection(connection, None).unwrap();
        let (_, resumed) = handle.execution_client().unwrap();
        assert!(handle.is_available());
        assert!(handle.key_is_current(resumed));
        assert!(!handle.key_is_current(generation));
        assert!(!handle.key_is_current(paused));
    }
}

#[tokio::test]
async fn preparation_runs_off_the_async_executor() {
    let caller = std::thread::current().id();
    let prepared = run_blocking_preparation(move || {
        assert_ne!(std::thread::current().id(), caller);
        Ok(42)
    })
    .await
    .unwrap();
    assert_eq!(prepared, 42);
}

#[tokio::test]
async fn cancelled_preparation_holds_its_slot_until_blocking_work_finishes() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let (started, mut starts) = tokio::sync::mpsc::unbounded_channel();
    let active = Arc::new(AtomicU64::new(0));
    let peak = Arc::new(AtomicU64::new(0));
    let mut tasks = Vec::new();
    for index in 0..PREPARATION_WORKERS + 1 {
        let gate = Arc::clone(&gate);
        let started = started.clone();
        let active = Arc::clone(&active);
        let peak = Arc::clone(&peak);
        tasks.push(tokio::spawn(async move {
            run_blocking_preparation(move || {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                started.send(index).unwrap();
                let (lock, release) = &*gate;
                let mut finished = lock.lock().unwrap();
                while !*finished {
                    finished = release.wait(finished).unwrap();
                }
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .await
        }));
    }
    let cancelled = starts.recv().await.unwrap();
    for _ in 1..PREPARATION_WORKERS {
        starts.recv().await.unwrap();
    }
    tasks[cancelled].abort();
    let held = tokio::time::timeout(Duration::from_millis(50), starts.recv())
        .await
        .is_err();
    let (lock, release) = &*gate;
    *lock.lock().unwrap() = true;
    release.notify_all();
    for (index, task) in tasks.into_iter().enumerate() {
        if index == cancelled {
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            task.await.unwrap().unwrap();
        }
    }
    assert!(held);
    assert_eq!(peak.load(Ordering::SeqCst), PREPARATION_WORKERS as u64);
}
