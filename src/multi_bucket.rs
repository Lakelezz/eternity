//! A multi-bucket type uses groups of buckets. Each group represents
//! a category, e.g. a command name or HTTP endpoint, which then contains
//! buckets. The buckets represent individual identities, such as the command
//! user's ID or the endpoint parameters.
//!
//! # Group and Bucket Keys
//! The group key accesses a bucket group and the bucket key accesses the
//! actual bucket instance containing the rate limit data within a group.
//!
//! # Limit Behaviour
//! Buckets are locked when accessed. Once a rate limit occurs, other
//! attempts on locking the bucket will await until the bucket is free again.
//!
//! # Generics
//! When defining your multi-bucket type, the first generic you set is the
//! group key. It decides the category of dispatch, e.g. a specific
//! bot command. <br>
//! The second generic you specify picks a rate limit measuering instance.
//! Therefore, if you want a global rate limit, you simply use `None` as
//! bucket key but when you want it to be limited per user, you provide the
//! user ID. <br>
//! Last but not least, when the `cache`-feature is enabled, you can pass
//! the optional third generic to decide what type of value you want to cache.
//!
//! # Usage
//! The recommended way of adding buckets to a multi-bucket is by calling
//! `build_group` on them. This returns a convenient builder API.
//!
//! The other option is to implement the [`ToBucket`]-trait and call
//! `insert_group` for each group. This can be useful if you want to dynamically
//! add new groups and adjust the bucket values based on its key's properties.
//! E.g. if the key has a field named `property` and it has a special
//! value, the bucket could have a different rate limit process.
//!
//! You may also construct the bucket manually and insert it via
//! `insert_bucket`. However `build_group` will do this for you already.
//!
//! [`ToBucket`]: ToBucket
use crate::{Bucket, BucketBuilder, RateLimitInfo};
use std::collections::HashMap;

use std::hash::Hash;
use std::mem::Discriminant;
use tokio::sync::Mutex;

#[cfg(feature = "cache")]
use std::collections::hash_map::Entry;
#[cfg(feature = "cache")]
use std::future::Future;

/// Creates a bucket using the key.
///
/// This trait ensures that all possible valid sets can become a bucket.
/// The `Value` resembles the cached value and is optional,
/// if caching is not needed, it can be be ignored.
pub trait ToBucket<Key, Value = ()>
where
    Key: Hash + PartialEq + Clone + Eq + Send + Sync,
    Value: Clone + Send,
{
    /// Turns a key into a bucket.
    fn to_bucket(&self) -> Option<Bucket<Key, Value>>;
}

/// This builder takes ownership of [`LimitedRequests`] and allows to add
/// a new bucket while providing a builder API to the user.
/// The taken [`LimitedRequests`] will be given back once [`build`] is
/// called.
///
/// This type is intended to be provided by [`LimitedRequests::build_group`]
///
/// [`build`]: Self::build
/// [`LimitedRequests`]: LimitedRequests
/// [`LimitedRequests::build_group`]: LimitedRequests::build_group
pub struct LimitedRequestsModifier<'a, GroupKey, BucketKey = GroupKey, Value = ()>
where
    GroupKey: Hash + PartialEq + Clone + Eq + Send + Sync,
    BucketKey: Hash + PartialEq + Clone + Eq + Send + Sync,
    Value: Clone + Send + Sync,
{
    group_key: GroupKey,
    multi_bucket: &'a mut LimitedRequests<GroupKey, BucketKey, Value>,
    bucket_builder: BucketBuilder,
}

impl<'a, GroupKey, BucketKey, Value> LimitedRequestsModifier<'a, GroupKey, BucketKey, Value>
where
    GroupKey: Hash + PartialEq + Clone + Eq + Send + Sync,
    BucketKey: Hash + PartialEq + Clone + Eq + Send + Sync,
    Value: Clone + Send + Sync,
{
    fn new(
        group_key: GroupKey,
        multi_bucket: &'a mut LimitedRequests<GroupKey, BucketKey, Value>,
    ) -> Self {
        Self {
            group_key,
            multi_bucket,
            bucket_builder: BucketBuilder::default(),
        }
    }

    /// The time to elapse between hitting the limiter.
    ///
    /// Expressed in seconds.
    #[inline]
    #[must_use]
    pub fn delay(mut self, secs: u64) -> Self {
        self.bucket_builder.delay(secs);

        self
    }

    /// How long the bucket will apply for.
    ///
    /// Expressed in seconds.
    #[inline]
    #[must_use]
    pub fn time_span(mut self, secs: u64) -> Self {
        self.bucket_builder.time_span(secs);

        self
    }

    /// Number of hits allowed per [`time_span`].
    ///
    /// [`time_span`]: Self::time_span
    #[inline]
    #[must_use]
    pub fn limit(mut self, n: u32) -> Self {
        self.bucket_builder.limit(n);

        self
    }

    /// If this is set to `true`, the invocation of the
    /// command will be delayed `amount` times instead of stopping command
    /// dispatch.
    ///
    /// By default this value is `false` and rate limits will cancel instead.
    #[inline]
    #[must_use]
    pub fn await_ratelimits(mut self, is_awaiting: bool) -> Self {
        self.bucket_builder.await_ratelimits(is_awaiting);

        self
    }

    /// Constructs the bucket, adds it to the related multi bucket and returns
    /// the mutated [`LimitedRequests`].
    pub fn build(mut self) {
        self.multi_bucket
            .insert_bucket(self.group_key, self.bucket_builder.build());
    }
}

/// This limiter can use a *group key* for each bucket group and a bucket key
/// to get the bucket instance.
///
/// A `GroupKey` can be a `String`, while the key for each instance of a bucket
/// can be a number.
///
/// By default, the `BucketKey` is the `GroupKey`.
///
/// # Example:
/// Use the command name as group key and the user ID as instance key.
/// You can use the same key for both too.
pub struct LimitedRequests<GroupKey, BucketKey = GroupKey, Value = ()>
where
    GroupKey: Hash + PartialEq + Clone + Eq + Send + Sync,
    BucketKey: Hash + PartialEq + Clone + Eq + Send + Sync,
    Value: Clone + Send + Sync,
{
    buckets: HashMap<GroupKey, Mutex<Bucket<BucketKey, Value>>>,
}

impl<GroupKey, BucketKey, Value> LimitedRequests<GroupKey, BucketKey, Value>
where
    GroupKey: Hash + PartialEq + Clone + Eq + Send + Sync,
    BucketKey: Hash + PartialEq + Clone + Eq + Send + Sync,
    Value: Clone + Send + Sync,
{
    /// Creates an empty limiter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }

    /// Inserts a single group by turning a `GroupKey` to a
    /// [`Bucket`](crate::Bucket). <br>
    ///
    /// It requires mutable ownership over `self`, if you know all groups
    /// before using this data structure, insert them ahead of time and
    /// use [`cache_or`](Self::cache_or). <br>
    /// This approach requires immutable access and prevents needing a locking
    /// mechanism.
    ///
    /// If you do not know all types beforehand, you will need a locking
    /// mechanism to call [`mut_cache_or`](Self::mut_cache_or) for the mutable
    /// ownership. <br>
    /// It allows you to add group buckets on the run.
    ///
    /// # Errors
    /// If a key cannot be converted, the method will return a `Vec` containing
    /// nonconvertible keys. This means, you have provided a key that cannot
    /// construct a bucket group.
    ///
    // `map_or`-solution won't compile due to moved `key` in `default`-closure.
    #[allow(clippy::option_if_let_else)]
    pub fn insert_group(&mut self, bucket_key: GroupKey) -> Result<(), GroupKey>
    where
        GroupKey: ToBucket<BucketKey, Value>,
    {
        if let Some(bucket) = bucket_key.to_bucket() {
            self.buckets.insert(bucket_key, Mutex::new(bucket));

            Ok(())
        } else {
            Err(bucket_key)
        }
    }

    /// Add a bucket group for the `bucket_key`.
    /// The method returns a new builder to add it to `self`.
    pub fn build_group(
        &mut self,
        bucket_key: GroupKey,
    ) -> LimitedRequestsModifier<GroupKey, BucketKey, Value> {
        LimitedRequestsModifier::new(bucket_key, self)
    }

    /// Inserts multiple groups at once using `GroupKey`s. <br>
    ///
    /// This method requires mutable ownership over `self`,
    /// if you know all groups before using this data structure,
    /// insert them ahead of time and use [`cache_or`](Self::cache_or).
    ///
    /// This approach requires immutable access and prevents needing a locking
    /// mechanism.
    ///
    /// If you do not know all types beforehand, you will need a locking
    /// mechanism to call [`mut_cache_or`](Self::mut_cache_or) for mutable
    /// ownership. <br>
    /// It allows you to add group buckets on the run.
    ///
    /// # Errors
    /// If a key cannot be converted, the method will return a `Vec` containing
    /// nonconvertible keys. This means, you have provided a key that cannot
    /// construct a bucket group.
    pub fn insert_groups<IterItem>(
        &mut self,
        bucket_keys: impl IntoIterator<Item = GroupKey>,
    ) -> Result<(), Vec<GroupKey>>
    where
        GroupKey: ToBucket<BucketKey, Value>,
    {
        let erroneous_keys: Vec<GroupKey> = bucket_keys
            .into_iter()
            .filter_map(|key| self.insert_group(key).err())
            .collect();

        if erroneous_keys.is_empty() {
            Ok(())
        } else {
            Err(erroneous_keys)
        }
    }

    /// Inserts a single `bucket` to the group with `group_key`.
    ///
    /// This method requires mutable ownership over `self`,
    /// if you know all groups before using this data structure,
    /// insert them ahead of time and use [`cache_or`](Self::cache_or).
    ///
    /// This approach requires immutable access and prevents needing a locking
    /// mechanism.
    ///
    /// If you do not know all types beforehand, you will need a locking
    /// mechanism to call [`mut_cache_or`](Self::mut_cache_or) for mutable
    /// ownership. <br>
    /// It allows you to add group buckets on the run.
    pub fn insert_bucket(&mut self, group_key: GroupKey, bucket: Bucket<BucketKey, Value>) {
        self.buckets.insert(group_key, Mutex::new(bucket));
    }

    /// Inserts multiple `buckets` to the group with `group_key`.
    ///
    /// This method requires mutable ownership over `self`,
    /// if you know all groups before using this data structure,
    /// insert them ahead of time and use [`cache_or`](Self::cache_or).
    ///
    /// This approach requires immutable access and prevents needing a locking
    /// mechanism.
    ///
    /// If you do not know all types beforehand, you will need a locking
    /// mechanism to call [`mut_cache_or`](Self::mut_cache_or) for mutable
    /// ownership. <br>
    /// It allows you to add group buckets on the run.
    pub fn insert_buckets(
        &mut self,
        group_key: &GroupKey,
        buckets: impl IntoIterator<Item = Bucket<BucketKey, Value>>,
    ) {
        buckets
            .into_iter()
            .for_each(|bucket| self.insert_bucket(group_key.clone(), bucket));
    }

    /// This method will return a rate limit once the conditions are met.
    /// However, the method does not cache anything if the cache feature is
    /// not enabled.
    pub async fn hit_limit(
        &self,
        bucket_key: &GroupKey,
        value_key: &BucketKey,
    ) -> Option<RateLimitInfo<Value>> {
        let bucket = self.buckets.get(bucket_key)?;

        bucket.lock().await.hit_limit(value_key).await
    }

    /// This method will access the cached value if limited *or* execute
    /// the given `function`.
    ///
    /// # Difference to [`mut_cache_or`](Self::cache_or)
    /// When the `group_key` does not exist, the group won't be created and
    /// `None` will be returned.
    ///
    /// # Cache Feature
    /// If the `function` is executed and caching is enabled,
    /// the returned value will be cached.
    #[cfg(feature = "cache")]
    pub async fn cache_or(
        &self,
        group_key: &GroupKey,
        bucket_key: &BucketKey,
        function: impl Future<Output = Option<Value>> + Send,
    ) -> Option<Value> {
        let group = self.buckets.get(group_key)?;

        let info = group.lock().await.hit_limit(bucket_key).await;

        if let Some(info) = info {
            info.cached
        } else {
            let to_cache = function.await;

            if let Some(ref to_cache) = to_cache {
                group
                    .lock()
                    .await
                    .add_cache_value(bucket_key, to_cache.clone())
                    .await;
            }

            to_cache
        }
    }

    /// This method will access the cached value if limited *or* execute
    /// the given `function`.
    ///
    /// # Difference to [`cache_or`](Self::cache_or)
    /// When the `group_key` does not exist, the group will be created.
    ///
    /// # Cache Feature
    /// If the `function` is executed and caching is enabled,
    /// the returned value will be cached.
    #[cfg(feature = "cache")]
    pub async fn mut_cache_or(
        &mut self,
        group_key: &GroupKey,
        bucket_key: &BucketKey,
        function: impl Future<Output = Option<Value>> + Send,
    ) -> Option<Value>
    where
        GroupKey: ToBucket<BucketKey, Value>,
    {
        // TODO: Replace with `Entry::or_insert_with_key` once stable.
        let group = match self.buckets.get(group_key) {
            Some(group) => group,
            None => match self.buckets.entry(group_key.clone()) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    let value = entry.key().to_bucket()?;
                    entry.insert(Mutex::new(value))
                }
            },
        };

        let info = group.lock().await.hit_limit(bucket_key).await;
        if let Some(info) = info {
            info.cached
        } else {
            let to_cache = function.await;

            if let Some(ref to_cache) = to_cache {
                group
                    .lock()
                    .await
                    .add_cache_value(bucket_key, to_cache.clone())
                    .await;
            }

            to_cache
        }
    }
}

impl<GroupKey, BucketKey, Value> Default for LimitedRequests<GroupKey, BucketKey, Value>
where
    GroupKey: Hash + PartialEq + Clone + Eq + Send + Sync,
    BucketKey: Hash + PartialEq + Clone + Eq + Send + Sync,
    Value: Clone + Send + Sync,
{
    fn default() -> Self {
        Self::new()
    }
}

/// This builder takes ownership of [`CachedLimitedEnums`] and allows to add
/// a new bucket while providing a builder API to the user.
/// The taken [`CachedLimitedEnums`] will be given back once [`build`] is
/// called.
///
/// This type is intended to be provided by [`CachedLimitedEnums::build_group`]
///
/// [`build`]: Self::build
/// [`CachedLimitedEnums`]: CachedLimitedEnums
/// [`CachedLimitedEnums::build_group`]: CachedLimitedEnums::build_group
pub struct CachedLimitedEnumsModifier<'a, Key, Value>
where
    Key: Hash + PartialEq + Clone + Eq + Send + Sync,
    Value: Clone + Send + Sync,
{
    key: &'a Key,
    multi_bucket: &'a mut CachedLimitedEnums<Key, Value>,
    bucket_builder: BucketBuilder,
}

impl<'a, 'b, Key, Value> CachedLimitedEnumsModifier<'a, Key, Value>
where
    Key: Hash + PartialEq + Clone + Eq + Send + Sync,
    Value: Clone + Send + Sync,
{
    fn new(key: &'a Key, multi_bucket: &'a mut CachedLimitedEnums<Key, Value>) -> Self {
        Self {
            key,
            multi_bucket,
            bucket_builder: BucketBuilder::default(),
        }
    }

    /// The time to elapse between hitting the limiter.
    ///
    /// Expressed in seconds.
    #[inline]
    #[must_use]
    pub fn delay(mut self, secs: u64) -> Self {
        self.bucket_builder.delay(secs);

        self
    }

    /// How long the bucket will apply for.
    ///
    /// Expressed in seconds.
    #[inline]
    #[must_use]
    pub fn time_span(mut self, secs: u64) -> Self {
        self.bucket_builder.time_span(secs);

        self
    }

    /// Number of hits allowed per [`time_span`].
    ///
    /// [`time_span`]: Self::time_span
    #[inline]
    #[must_use]
    pub fn limit(mut self, n: u32) -> Self {
        self.bucket_builder.limit(n);

        self
    }

    /// If this is set to `true`, the invocation of the
    /// command will be delayed `amount` times instead of stopping command
    /// dispatch.
    ///
    /// By default this value is `false` and rate limits will cancel instead.
    #[inline]
    #[must_use]
    pub fn await_ratelimits(mut self, is_awaiting: bool) -> Self {
        self.bucket_builder.await_ratelimits(is_awaiting);

        self
    }

    /// Constructs the bucket, adds it to the related multi bucket and returns
    /// the mutated [`LimitedRequests`].
    pub fn build(mut self) {
        self.multi_bucket
            .insert_bucket(self.key, self.bucket_builder.build());
    }
}

/// # Disclaimer:
/// The `Key` is expected to be an enum!
/// If your key is no enum, use [`LimitedRequests`](self::LimitedRequests).
///
/// # Implementation
/// This type uses `std::mem::discriminant` on the key to differentiate between
/// bucket groups.
///
/// It's tailored to limit calling and caching HTTP endpoints, where the
/// the key represents the endpoints containing arguments.
/// At first, the type will convert the key to a discriminant to pick the
/// the bucket group, ignoring the fields, and then it will use the enum
/// with fields to find the actual bucket instance.
///
/// # **Warning**
/// As said, if the key is not an enum, this type will not work as expected. <br>
/// Use [`LimitedRequests`](self::LimitedRequests) instead for generic keys.
#[derive(Default)]
pub struct CachedLimitedEnums<Key, Value>
where
    Key: Hash + PartialEq + Clone + Eq + Send + Sync,
    Value: Clone + Send + Sync,
{
    buckets: HashMap<Discriminant<Key>, Mutex<Bucket<Key, Value>>>,
}

impl<Key, Value> CachedLimitedEnums<Key, Value>
where
    Key: Hash + PartialEq + Clone + Eq + Send + Sync,
    Value: Clone + Send + Sync,
{
    /// Create an empty [`CachedLimitedEnums`](self::CachedLimitedEnums).
    #[must_use]
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }

    /// Inserts a single group by turning a `GroupKey` to a
    /// [`Bucket`](crate::Bucket). <br>
    /// Returns an `Err` with a `GroupKey` if it failed converting to `None`.
    ///
    /// It requires mutable ownership over `self`, if you know all groups
    /// before using this data structure, insert them ahead of time and
    /// use [`cache_or`](Self::cache_or). <br>
    /// This approach requires immutable access and prevents needing a locking
    /// mechanism.
    ///
    /// If you do not know all types beforehand, you will need a locking
    /// mechanism to call [`mut_cache_or`](Self::mut_cache_or) for the mutable
    /// ownership. <br>
    /// It allows you to add group buckets on the run.
    ///
    /// # Errors
    /// If a key cannot be converted, the method will return the
    /// nonconvertible keys. This means, you have provided a key that cannot
    /// construct a bucket group.
    ///
    // This type explicitly documents that non-enums are not supported.
    // There is no way to check whether a type is an enum in Rust yet.
    #[allow(clippy::mem_discriminant_non_enum)]
    // `map_or`-solution won't compile due to moved `key` in `default`-closure.
    #[allow(clippy::option_if_let_else)]
    pub fn insert_enum(&mut self, key: Key) -> Result<(), Key>
    where
        Key: ToBucket<Key, Value>,
    {
        if let Some(bucket) = key.to_bucket() {
            self.buckets
                .insert(std::mem::discriminant(&key), Mutex::new(bucket));

            Ok(())
        } else {
            Err(key)
        }
    }

    /// Inserts an already created bucket for a `key`.
    #[allow(clippy::mem_discriminant_non_enum)]
    pub fn insert_bucket(&mut self, key: &Key, bucket: Bucket<Key, Value>) {
        self.buckets
            .insert(std::mem::discriminant(key), Mutex::new(bucket));
    }

    /// Inserts an already created bucket for a `key`.
    ///
    /// Add a bucket group for the `bucket_key`.
    /// The method returns a new builder to add it to `self`.
    #[allow(clippy::mem_discriminant_non_enum)]
    pub fn build_group<'a>(
        &'a mut self,
        key: &'a Key,
    ) -> CachedLimitedEnumsModifier<'a, Key, Value> {
        CachedLimitedEnumsModifier::new(key, self)
    }

    /// Inserts multiple *enums*. The type must be an enum and nothing else.
    ///
    /// This method requires mutable ownership over `self`,
    /// if you know all enum before using this data structure,
    /// insert them ahead of time and use [`cache_or`](Self::cache_or).
    ///
    /// This approach requires immutable access and prevents needing a locking
    /// mechanism.
    ///
    /// If you do not know all types beforehand, you will need a locking
    /// mechanism to call [`mut_cache_or`](Self::mut_cache_or) for mutable
    /// ownership. <br>
    /// It allows you to add group buckets on the run.
    ///
    /// In order to provide automatic coverage for your enum's variants,
    /// try the [`strum::EnumIter`](https://docs.rs/strum/0.20.0/strum/derive.EnumIter.html)
    /// macro.
    ///
    /// # Errors
    /// If a key cannot be converted, the method will return a `Vec` containing
    /// nonconvertible keys. This means, you have provided a key that cannot
    /// construct a bucket group.
    pub fn insert_enums(&mut self, keys: impl IntoIterator<Item = Key>) -> Result<(), Vec<Key>>
    where
        Key: ToBucket<Key, Value>,
    {
        let erroneous_keys: Vec<Key> = keys
            .into_iter()
            .filter_map(|key| self.insert_enum(key).err())
            .collect();

        if erroneous_keys.is_empty() {
            Ok(())
        } else {
            Err(erroneous_keys)
        }
    }

    /// This method will return a rate limit once the conditions are met.
    /// However, the method does not cache anything if the cache feature is
    /// not enabled.
    ///
    /// # Warning
    /// The key may **only be an enum** and no other type.
    // This type explicitly documents that non-enums are not supported.
    // There is no way to check whether a type is an enum in Rust yet.
    #[allow(clippy::mem_discriminant_non_enum)]
    pub async fn hit_limit(&mut self, key: &Key) -> Option<RateLimitInfo<Value>> {
        let bucket = self.buckets.get(&std::mem::discriminant(key))?;

        bucket.lock().await.hit_limit(key).await
    }

    /// This method will access the cached value if rate limited *or* execute
    /// the given `function`.
    ///
    /// # Difference to [`mut_cache_or`](Self::mut_cache_or)
    /// When the `group_key` does not exist, the group won't be created and
    /// `None` will be returned.
    ///
    /// # Cache Feature
    /// If the `function` is executed and caching is enabled,
    /// the returned value will be cached.
    ///
    /// # Warning
    /// The key may **only be an enum** and no other type.
    // This type explicitly documents that non-enums are not supported.
    // There is no way to check whether a type is an enum in Rust yet.
    #[allow(clippy::mem_discriminant_non_enum)]
    #[cfg(feature = "cache")]
    pub async fn cache_or(
        &self,
        key: &Key,
        function: impl Future<Output = Option<Value>> + Send,
    ) -> Option<Value> {
        let group = self.buckets.get(&std::mem::discriminant(key))?;

        let info = group.lock().await.hit_limit(key).await;

        if let Some(info) = info {
            info.cached
        } else {
            let to_cache = function.await;

            if let Some(ref to_cache) = to_cache {
                group
                    .lock()
                    .await
                    .add_cache_value(key, to_cache.clone())
                    .await;
            }

            to_cache
        }
    }

    /// This method will access the cached value if limited *or* execute
    /// the given `function`.
    ///
    /// # Difference to [`cache_or`](Self::cache_or)
    /// When the `group_key` does not exist, the group will be created and
    /// and the `function` will be executed.
    ///
    /// # Cache Feature
    /// If the `function` is executed and caching is enabled,
    /// the returned value will be cached.
    ///
    /// # Warning
    /// The key may **only be an enum** and no other type.
    // This type explicitly documents that non-enums are not supported.
    // There is no way to check whether a type is an enum in Rust yet.
    #[allow(clippy::mem_discriminant_non_enum)]
    #[cfg(feature = "cache")]
    pub async fn mut_cache_or(
        &mut self,
        key: &Key,
        function: impl Future<Output = Option<Value>> + Send,
    ) -> Option<Value>
    where
        Key: ToBucket<Key, Value>,
    {
        let bucket = key.to_bucket()?;
        let group = self
            .buckets
            .entry(std::mem::discriminant(key))
            .or_insert_with(|| Mutex::new(bucket));

        let info = group.lock().await.hit_limit(key).await;
        if let Some(info) = info {
            info.cached
        } else {
            let to_cache = function.await;

            if let Some(ref to_cache) = to_cache {
                group
                    .lock()
                    .await
                    .add_cache_value(key, to_cache.clone())
                    .await;
            }

            to_cache
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This test will use an enum where each variant requires specific
    /// rate limiting.
    #[tokio::test]
    async fn test_routed_caching() {
        use crate::BucketBuilder;
        use ToBucket;

        #[derive(Hash, PartialEq, Eq, Clone)]
        enum SpecificRoute {
            GetUser(u64),
            GetAllUsers,
        };

        impl ToBucket<SpecificRoute, String> for SpecificRoute {
            fn to_bucket(&self) -> Option<Bucket<SpecificRoute, String>> {
                Some(match self {
                    // Shorter rate limit.
                    Self::GetUser(_) => BucketBuilder::new().limit(2).time_span(60).build(),
                    // Longer rate limit.
                    Self::GetAllUsers => BucketBuilder::new().limit(1).time_span(600).build(),
                })
            }
        }

        let mut buckets: CachedLimitedEnums<SpecificRoute, String> = CachedLimitedEnums::new();

        // Expected: Calls the function instead of returning the cached value.
        let value = buckets
            .mut_cache_or(&SpecificRoute::GetUser(1), async move {
                Some("Ferris".to_string())
            })
            .await;

        assert_eq!(value, Some("Ferris".to_string()));

        // Expected: Calls the function instead of returning the cached value.
        let value = buckets
            .mut_cache_or(&SpecificRoute::GetUser(1), async move {
                Some("Ferris2".to_string())
            })
            .await;

        assert_eq!(value, Some("Ferris2".to_string()));

        // Expected: Calls the function instead of returning the cached value.
        let value = buckets
            .mut_cache_or(&SpecificRoute::GetAllUsers, async move {
                Some("Ferris, Ferris2".to_string())
            })
            .await;

        assert_eq!(value, Some("Ferris, Ferris2".to_string()));

        // Expected: Returns the cached value instead of calling the function.
        let value = buckets
            .mut_cache_or(&SpecificRoute::GetAllUsers, async move {
                Some("Ferris, Ferris2, Ferris3".to_string())
            })
            .await;

        assert_eq!(value, Some("Ferris, Ferris2".to_string()));
    }
}
