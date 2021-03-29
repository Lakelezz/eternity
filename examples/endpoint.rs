//! This example showcases how to the enum based multi-bucket.
//! The enum's fields are ignored, turning the variant into the group key.
//!
//! The example will showcase two different ways to add groups too:
//! Approach #1 will use a builder API, a single simple step.
//! Approach two will use the `ToBucket`-trait hence requiring to implement the
//! trait and then adding the groups. The crate `strum` offers us a lot of
//! convenience here.
//!
use eternity::multi_bucket::CachedLimitedEnums;
use eternity::multi_bucket::ToBucket;
use eternity::Bucket;
use eternity::BucketBuilder;

use strum::IntoEnumIterator;
use strum_macros::EnumIter;

#[derive(EnumIter, Debug, Eq, Hash, PartialEq, Clone)]
enum Endpoint {
    GetUser(u64),
    GetUsers,
    GetChannels,
}

// Needed for approach #2, using the `ToBucket`-trait.
// This will implicitly convert our command name into a bucket.
impl ToBucket<Endpoint, String> for Endpoint {
    fn to_bucket(&self) -> Option<Bucket<Endpoint, String>> {
        Some(match self {
            // We allow 1 request in 10 seconds and queue requests
            // until the rate limit ends.
            Self::GetUser(_) => BucketBuilder::new()
                .limit(1)
                .time_span(10)
                .await_ratelimits(true)
                .build(),
            // We allow 1 request in 60 seconds, once the rate limit is reached,
            // further requests will be rejected and receive the cached value.
            Self::GetUsers => BucketBuilder::new()
                .limit(1)
                .time_span(60)
                .await_ratelimits(false)
                .build(),
            // We leave out the last variant to showcase it via approach #1.
            _ => unimplemented!(),
        })
    }
}

#[tokio::main]
async fn main() {
    let mut limiter = CachedLimitedEnums::new();
    limiter
        .insert_enums(vec![
            Endpoint::GetUsers,
            Endpoint::GetUser(Default::default()),
        ])
        .expect("Keys failed converting");

    limiter
        .build_group(&Endpoint::GetChannels)
        .limit(1)
        .time_span(120)
        .build();

    let client = Client { limiter };

    dbg!(client.get_user(1).await);
    dbg!(client.get_user(1).await);
    dbg!(client.get_user(2).await);

    // Instead of manually adding each enum-variant, we can use an external
    // crate to automatically cover all variants.
    // Let's use `EnumIter` to instantiate now:
    let mut limiter = CachedLimitedEnums::new();
    limiter
        .insert_enums(Endpoint::iter())
        .expect("Keys failed converting");
}

struct Client {
    limiter: CachedLimitedEnums<Endpoint, String>,
}

impl Client {
    async fn get_user(&self, user_id: u64) -> Option<String> {
        self.limiter
            .cache_or(&Endpoint::GetUser(user_id), async move {
                println!("Called expensive request.");

                Some(format!("eternity-{}", user_id))
            })
            .await
    }
}
