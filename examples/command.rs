//! This example showcases how commands can be limited.
//!
//! The example will showcase two different ways to add groups too:
//! Approach #1 will use a builder API, a single simple step.
//! Approach two will use the `ToBucket`-trait hence requiring to implement the
//! trait and then adding the groups.
//!
use eternity::multi_bucket::LimitedRequests;
use eternity::multi_bucket::ToBucket;
use eternity::Bucket;
use eternity::BucketBuilder;

#[derive(Eq, Hash, PartialEq, Clone)]
struct UserId(u64);

// Needed for approach #2, using the `ToBucket`-trait.
// This will implicitly convert our command name into a bucket.
impl ToBucket<UserId> for String {
    fn to_bucket(&self) -> Option<Bucket<UserId>> {
        // We allow 4 requests in 60 seconds and expect a 1 second delay minimum.
        Some(
            BucketBuilder::new()
                .limit(4)
                .time_span(60)
                .delay(1)
                .await_ratelimits(true)
                .build(),
        )
    }
}

#[tokio::main]
async fn main() {
    let mut limiter: LimitedRequests<String, UserId> = LimitedRequests::new();

    // Approach #1:
    // Use the builder returned by `build_group`.
    // We allow 4 requests in 60 seconds and expect a 1 second delay minimum.
    limiter
        .build_group("funny_gif".to_string())
        .limit(4)
        .time_span(60)
        .delay(1)
        .await_ratelimits(true)
        .build();

    // Approach #2:
    // Use the `ToBucket`-trait.
    let _ = dbg!(limiter.insert_group("funny_gif".to_string()));

    let user = UserId(1);
    let command = "funny_gif".to_string();

    for iteration in 1..=5 {
        // Round 5 will exceed the rate limit and take about 10 seconds
        // to return.
        dbg!("Round ", iteration);
        dbg!(limiter.hit_limit(&command, &user).await);
    }
}
