use super::future::ResponseFuture;

use tower_service::Service;

use futures_core::ready;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio_sync::semaphore::{self, Semaphore};

/// Enforces a limit on the concurrent number of requests the underlying
/// service can handle.
#[derive(Debug)]
pub struct ConcurrencyLimit<T> {
    inner: T,
    limit: Limit,
}

#[derive(Debug)]
struct Limit {
    semaphore: Arc<Semaphore>,
    permit: semaphore::Permit,
}

impl<T> ConcurrencyLimit<T> {
    /// Create a new concurrency limiter.
    pub fn new(inner: T, max: usize) -> Self {
        ConcurrencyLimit {
            inner,
            limit: Limit {
                semaphore: Arc::new(Semaphore::new(max)),
                permit: semaphore::Permit::new(),
            },
        }
    }

    /// Get a reference to the inner service
    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Get a mutable reference to the inner service
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Consume `self`, returning the inner service
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<S, Request> Service<Request> for ConcurrencyLimit<S>
where
    S: Service<Request>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.limit.permit.poll_acquire(cx, &self.limit.semaphore))
            .expect("poll_acquire after semaphore closed ");

        Poll::Ready(ready!(self.inner.poll_ready(cx)))
    }

    fn call(&mut self, request: Request) -> Self::Future {
        // Make sure a permit has been acquired
        if self
            .limit
            .permit
            .try_acquire(&self.limit.semaphore)
            .is_err()
        {
            panic!("max requests in-flight; poll_ready must be called first");
        }

        // Call the inner service
        let future = self.inner.call(request);

        // Forget the permit, the permit will be returned when
        // `future::ResponseFuture` is dropped.
        self.limit.permit.forget();

        ResponseFuture::new(future, self.limit.semaphore.clone())
    }
}

impl<S> Clone for ConcurrencyLimit<S>
where
    S: Clone,
{
    fn clone(&self) -> ConcurrencyLimit<S> {
        ConcurrencyLimit {
            inner: self.inner.clone(),
            limit: Limit {
                semaphore: self.limit.semaphore.clone(),
                permit: semaphore::Permit::new(),
            },
        }
    }
}

impl Drop for Limit {
    fn drop(&mut self) {
        self.permit.release(&self.semaphore);
    }
}
