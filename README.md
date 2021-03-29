[![ci-badge][]][ci] [![docs-badge][]][docs] [![crates.io version]][crates.io link] [![rust 1.48.0+ badge]][rust 1.48.0+ link]

# Eternity

Eternity is a Rust library to rate limit and optionally cache keyed results.

Two use cases:
- You interact with a REST-API lacking official rate limits and you want to
cache frequent requests.
- You have a bot with commands and you want to ratelimit them per user or even globally.

View the [examples] on how to use this library for these cases.

# Example

A basic limiter for endpoints:

```rust,no_run
use eternity::multi_bucket::{CachedLimitedEnums, ToBucket};

#[derive(Hash, PartialEq, Clone, Eq)]
enum Route {
    GetUser(u64),
    GetStats,
    GetGuild(u64),
}

#[tokio::main]
async fn main() {
    let mut limiter: CachedLimitedEnums<Route, String> = CachedLimitedEnums::new();
    limiter.build_group(&Route::GetUser(0)).limit(4).time_span(10).build();
    limiter.build_group(&Route::GetStats).limit(10).time_span(60).build();
    limiter.build_group(&Route::GetGuild(0)).limit(1).time_span(60).build();

    let result = limiter.cache_or(&Route::GetUser(1), get_user(1));
}

async fn get_user(user_id: u64) -> Option<String> {
    Some(format!("eternity-{}", user_id))
}

```

### All Examples

Here are [examples] to see what this library can be used for.

### Features

There are two features and they are not enabled by default.

- `cache`: Enables functionality to cache values.
- `tokio_0_2`: By default this crate supports `tokio` `v1`, this feature
enables `v0.2` support.

# Installation

Add the following to your `Cargo.toml` file:

```toml
[dependencies]
eternity = "0.1"
```

Eternity supports a minimum of Rust 1.48.

[ci]: https://github.com/Lakelezz/eternity/actions
[ci-badge]: https://img.shields.io/github/workflow/status/Lakelezz/eternity/CI?style=flat-square

[crates.io link]: https://crates.io/crates/eternity
[crates.io version]: https://img.shields.io/crates/v/eternity.svg?style=flat-square

[docs]: https://docs.rs/eternity
[docs-badge]: https://img.shields.io/badge/docs-online-5023dd.svg?style=flat-square
[examples]: https://github.com/Lakelezz/eternity/tree/current/examples

[logo]: https://raw.githubusercontent.com/Lakelezz/eternity/current/logo.png
[rust 1.48.0+ badge]: https://img.shields.io/badge/rust-1.48.0+-93450a.svg?style=flat-square
[rust 1.48.0+ link]: https://blog.rust-lang.org/2020/11/19/Rust-1.48.html
