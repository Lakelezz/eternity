//! This crate ensures rate limits and can return cached results instead.
//!
//! Caching is an optional mechanism helping to avoid repeated expensive
//! procedures, such as REST requests and calcuations, until certain time
//! elapsed.
//!
//! Nonetheless, as it's an optional feature, one can simply use the
//! rate limiter for nothing but throttling, e.g. bot commands, without
//! using memory on caching.
//!
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![deny(clippy::cargo)]
#![deny(missing_docs)]

use std::{
    collections::HashMap,
    hash::Hash,
    time::{Duration, Instant},
};

#[cfg(not(feature = "cache"))]
use std::marker::PhantomData;

#[cfg(feature = "cache")]
use std::future::Future;

pub mod multi_bucket;

#[cfg(all(feature = "tokio_0_2", not(feature = "tokio")))]
extern crate tokio_compat as tokio;

#[cfg(all(feature = "tokio_0_2", not(feature = "tokio")))]
use tokio::time::delay_for as sleep;

#[cfg(all(feature = "tokio"))]
use tokio::time::sleep;

/// Action taken for the command invocation.
#[derive(Clone, Debug)]
pub enum RateLimitAction {
    /// Invocation has been delayed.
    Delayed,
    /// Tried to delay invocation but maximum of delays reached.
    FailedDelay,
    /// Cancelled the invocation due to time or ticket reasons.
    Cancelled,
}

/// This type checks for rate limits on `Key`s. <br>
/// If caching is enabled, it will cache values.
pub struct Bucket<Key, Value = ()>
where
    Key: Hash + PartialEq + Clone + Eq + Send + Sync,
    Value: Clone + Send,
{
    pub(crate) ratelimit: RateLimitSpec,
    pub(crate) tickets_for: HashMap<Key, RateLimitInstance<Value>>,
    pub(crate) await_ratelimits: bool,
}

impl<Key: Hash + PartialEq + Clone + Eq + Send + Sync, Value: Clone + Send> Bucket<Key, Value> {
    /// Tries to perform a hit in the bucket.
    /// If no hit can be performed, this method will return the rate limit.
    /// Otherwise, the method will return `None`.
    pub async fn hit_limit(&mut self, key: &Key) -> Option<RateLimitInfo<Value>> {
        let now = Instant::now();
        let Self {
            tickets_for,
            ratelimit,
            ..
        } = self;

        // The problem when using `entry` only is its expecting ownership of
        // the key, which forces the key to be cloned on every look-up or the
        // `hit_limit`  signature to demand an owned key, forcing the user to
        // clone.
        //
        // A key may be cheap, but there is no gurantee that it is.
        // For example: `u64` vs `String`.
        //
        // TODO: Replace the entire next block with `raw_entry` once it is on
        // stable Rust.
        //
        // https://doc.rust-lang.org/std/collections/struct.HashMap.html#method.raw_entry
        let ticket_owner = match tickets_for.get_mut(key) {
            Some(bucket) => bucket,
            None => tickets_for
                .entry(key.clone())
                .or_insert_with(|| RateLimitInstance::new(now)),
        };

        // Check if too many tickets have been taken already.
        // If all tickets are exhausted, return the needed delay
        // for this invocation.
        if let Some((timespan, limit)) = ratelimit.limit {
            if (ticket_owner.tickets + 1) > limit {
                if let Some(ratelimit) =
                    (ticket_owner.set_time + timespan).checked_duration_since(now)
                {
                    return Self::rating(ticket_owner, ratelimit, self.await_ratelimits).await;
                } else {
                    ticket_owner.tickets = 0;
                    ticket_owner.set_time = now;
                    #[cfg(feature = "cache")]
                    {
                        ticket_owner.cached_value = None;
                    }
                }
            }
        }

        // Check if `ratelimit.delay`-time passed between the last and
        // the current invocation
        // If the time did not pass, return the needed delay for this
        // invocation.
        if let Some(ratelimit) = ticket_owner
            .last_time
            .and_then(|x| (x + ratelimit.delay).checked_duration_since(now))
        {
            return Self::rating(ticket_owner, ratelimit, self.await_ratelimits).await;
        } else {
            ticket_owner.awaiting = ticket_owner.awaiting.saturating_sub(1);
            ticket_owner.tickets += 1;
            ticket_owner.is_first_try = true;
            ticket_owner.last_time = Some(now);
            #[cfg(feature = "cache")]
            {
                ticket_owner.cached_value = None;
            }
        }

        None
    }

    /// First, checks the bucket for a potential rate limit.
    /// If a rate limit applies, it will clone the cached value.
    /// If no value is cached, this method returns `None`.
    /// If no rate limit applies, this method will run `function` and
    /// if some value is returned by that function, caches the value.
    #[cfg(feature = "cache")]
    pub async fn hit_or_cache(
        &mut self,
        key: &Key,
        function: impl Future<Output = Option<Value>> + Send,
    ) -> Option<Value> {
        if let Some(cached_value) = self.hit_limit(key).await.and_then(|i| i.cached) {
            Some(cached_value)
        } else {
            let to_cache = function.await?;
            let value = to_cache.clone();
            self.add_cache_value(key, value).await;

            Some(to_cache)
        }
    }

    /// Inserts a cache value into the group of `key`.
    #[cfg(feature = "cache")]
    pub async fn add_cache_value(&mut self, key: &Key, value: Value) {
        if let Some(instance) = self.tickets_for.get_mut(key) {
            instance.cached_value = Some(value);
        }
    }

    async fn rating(
        ticket_owner: &mut RateLimitInstance<Value>,
        ratelimit: Duration,
        await_ratelimits: bool,
    ) -> Option<RateLimitInfo<Value>> {
        let was_first_try = ticket_owner.is_first_try;

        // Are delay limits left?
        let action = if await_ratelimits {
            RateLimitAction::Delayed
        } else {
            RateLimitAction::Cancelled
        };

        if let RateLimitAction::Delayed = action {
            sleep(ratelimit).await;

            return None;
        }

        Some(RateLimitInfo {
            rate_limit: ratelimit,
            active_delays: ticket_owner.awaiting,
            #[cfg(feature = "cache")]
            cached: ticket_owner.cached_value.clone(),
            action,
            is_first_try: was_first_try,
            #[cfg(not(feature = "cache"))]
            phantom: PhantomData,
        })
    }
}
/// A rate limit specification, where `delay` defines how much time must elapse
/// between each hit and the `limit` defines how long the rate limit applies
/// and the maximum amount of hits.
pub(crate) struct RateLimitSpec {
    /// The time between each hit to elapse.
    pub delay: Duration,
    /// A tuple consisting of first the duration for the rate limit to apply
    /// and second the maximum of hits until the rate limit is triggered.
    pub limit: Option<(Duration, u32)>,
}

/// An active rate limit record.
pub(crate) struct RateLimitInstance<Value> {
    /// Last time the instance has been hit.
    pub last_time: Option<Instant>,
    /// The time the instance has been created.
    pub set_time: Instant,
    /// The amount of hits that can be taken.
    pub tickets: u32,
    /// The amount of
    pub awaiting: u32,
    /// Whether this rate limit has been hit while it is active.
    pub is_first_try: bool,
    /// The cached value.
    #[cfg(feature = "cache")]
    pub cached_value: Option<Value>,
    #[cfg(not(feature = "cache"))]
    phantom: PhantomData<Value>,
}

impl<Value> RateLimitInstance<Value> {
    const fn new(creation_time: Instant) -> Self {
        Self {
            last_time: None,
            set_time: creation_time,
            tickets: 0,
            awaiting: 0,
            is_first_try: true,
            #[cfg(feature = "cache")]
            cached_value: None,
            #[cfg(not(feature = "cache"))]
            phantom: PhantomData,
        }
    }
}

/// Describes the rate limit encountered.
///
/// If this value is returned, the callee informs about a rate limit.
///
/// The term `hit` represents one attempt to hit the rate limit.
#[derive(Clone, Debug)]
pub struct RateLimitInfo<Value: Clone> {
    /// Time to elapse in order to invoke a command again.
    pub rate_limit: Duration,
    /// Amount of active delays by this target.
    pub active_delays: u32,
    /// Whether this is the first time the rate limit info has been
    /// returned for the bucket without the rate limit to elapse.
    pub is_first_try: bool,
    /// Action taken for this rate limit.
    /// `Delay` never occurs, as a value is returned after awaiting the delay.
    pub action: RateLimitAction,
    /// If a value has been cached, this field will yield a value.
    #[cfg(feature = "cache")]
    pub cached: Option<Value>,
    #[cfg(not(feature = "cache"))]
    phantom: PhantomData<Value>,
}

/// Builds a [`Bucket`](crate::Bucket).
///
/// The term `hit` represents one attempt to hit the rate limit.
pub struct BucketBuilder {
    pub(crate) delay: Duration,
    pub(crate) time_span: Duration,
    pub(crate) limit: u32,
    pub(crate) await_ratelimits: bool,
}

impl Default for BucketBuilder {
    fn default() -> Self {
        Self {
            delay: Duration::default(),
            time_span: Duration::default(),
            limit: 1,
            await_ratelimits: false,
        }
    }
}

impl BucketBuilder {
    /// A bucket collecting tickets per command invocation.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The time to elapse between hitting the limiter.
    ///
    /// Expressed in seconds.
    #[inline]
    pub fn delay(&mut self, secs: u64) -> &mut Self {
        self.delay = Duration::from_secs(secs);

        self
    }

    /// How long the bucket will apply for.
    ///
    /// Expressed in seconds.
    #[inline]
    pub fn time_span(&mut self, secs: u64) -> &mut Self {
        self.time_span = Duration::from_secs(secs);

        self
    }

    /// Number of hits allowed per [`time_span`].
    ///
    /// [`time_span`]: Self::time_span
    #[inline]
    pub fn limit(&mut self, n: u32) -> &mut Self {
        self.limit = n;

        self
    }

    /// If this is set to `true`, the invocation of the
    /// command will be delayed `amount` times instead of stopping command
    /// dispatch.
    ///
    /// By default this value is `false` and rate limits will cancel instead.
    #[inline]
    pub fn await_ratelimits(&mut self, is_awaiting: bool) -> &mut Self {
        self.await_ratelimits = is_awaiting;

        self
    }

    /// Constructs the bucket.
    #[inline]
    pub fn build<Key, Value>(&mut self) -> Bucket<Key, Value>
    where
        Key: Hash + PartialEq + Clone + Eq + Send + Sync,
        Value: Clone + Send,
    {
        Bucket {
            ratelimit: RateLimitSpec {
                delay: self.delay,
                limit: Some((self.time_span, self.limit)),
            },
            tickets_for: HashMap::new(),
            await_ratelimits: self.await_ratelimits,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_caching() {
        #[derive(Clone, Hash, PartialEq, Eq)]
        enum Route {
            GetUser(u64),
        };

        let mut bucket: Bucket<Route, String> =
            BucketBuilder::new().limit(2).time_span(60).delay(5).build();

        let value = bucket
            .hit_or_cache(
                &Route::GetUser(1),
                async move { Some("success1".to_string()) },
            )
            .await;

        assert_eq!(value, Some("success1".to_string()));

        let value = bucket
            .hit_or_cache(
                &Route::GetUser(1),
                async move { Some("success2".to_string()) },
            )
            .await;

        assert_eq!(value, Some("success1".to_string()));
    }
}
