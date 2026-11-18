//! Coalesces identical in-flight requests into one upstream call.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;

use futures::FutureExt as _;
use futures::future::{BoxFuture, Shared};

use crate::DataError;

type Flight<V> = Shared<BoxFuture<'static, Result<V, DataError>>>;

struct State<K, V> {
    next_id: u64,
    flights: HashMap<K, (u64, Flight<V>)>,
}

pub(crate) struct SingleFlight<K, V> {
    inflight: Mutex<State<K, V>>,
}

impl<K: Eq + Hash + Clone, V: Clone> SingleFlight<K, V> {
    pub fn new() -> Self {
        Self {
            inflight: Mutex::new(State {
                next_id: 0,
                flights: HashMap::new(),
            }),
        }
    }

    /// Runs `make()` for `key` unless a call for `key` is already running, in
    /// which case this caller waits for that one's result.
    pub async fn run<F>(&self, key: K, make: impl FnOnce() -> F) -> Result<V, DataError>
    where
        F: Future<Output = Result<V, DataError>> + Send + 'static,
    {
        let (id, flight) = {
            let mut guard = self
                .inflight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match guard.flights.get(&key) {
                Some((id, flight)) => (*id, flight.clone()),
                None => {
                    guard.next_id += 1;
                    let id = guard.next_id;
                    let flight = make().boxed().shared();
                    guard.flights.insert(key.clone(), (id, flight.clone()));
                    (id, flight)
                }
            }
        };
        let result = flight.await;
        // Remove our entry only: a newer flight for the same key must stay.
        let mut guard = self
            .inflight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard
            .flights
            .get(&key)
            .is_some_and(|(current, _)| *current == id)
        {
            guard.flights.remove(&key);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn concurrent_callers_share_one_call() {
        let flight = Arc::new(SingleFlight::<&'static str, u32>::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..10 {
            let (flight, calls) = (Arc::clone(&flight), Arc::clone(&calls));
            tasks.push(tokio::spawn(async move {
                flight
                    .run("k", || async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(40)).await;
                        Ok(7)
                    })
                    .await
            }));
        }
        for t in tasks {
            assert_eq!(t.await.unwrap(), Ok(7));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn errors_are_shared_and_the_next_call_starts_fresh() {
        let flight = SingleFlight::<u8, u32>::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        let first = flight
            .run(1, || async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(DataError::Timeout)
            })
            .await;
        assert_eq!(first, Err(DataError::Timeout));
        let c = Arc::clone(&calls);
        let second = flight
            .run(1, || async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(1)
            })
            .await;
        assert_eq!(second, Ok(1));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn different_keys_do_not_share() {
        let flight = SingleFlight::<u8, u32>::new();
        let a = flight.run(1, || async { Ok(1) }).await;
        let b = flight.run(2, || async { Ok(2) }).await;
        assert_eq!((a, b), (Ok(1), Ok(2)));
    }
}
