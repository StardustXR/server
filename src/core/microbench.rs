//! Minimal timing harness for in-crate micro-benchmarks.
//!
//! Benchmarks live next to the code they measure as `#[ignore]`d tests, so they can
//! use private APIs and the `test_new` constructors, and never run in a normal
//! `cargo test`. Run them (optimized, without release's LTO link times) with:
//!
//! ```sh
//! cargo test --profile benching -- --ignored bench_ --nocapture --test-threads=1
//! ```

use std::hint::black_box;
use std::time::{Duration, Instant};

const BATCH_TARGET: Duration = Duration::from_millis(50);
const SAMPLES: u32 = 5;

/// Time `f` and print per-iteration cost. The batch size is grown until one batch
/// takes ≥ [`BATCH_TARGET`], then [`SAMPLES`] batches are measured; `best` is the
/// least-noisy estimate, `mean` shows spread.
pub fn bench<T>(name: &str, mut f: impl FnMut() -> T) {
	fn time_batch<T>(iters: u64, f: &mut impl FnMut() -> T) -> Duration {
		let start = Instant::now();
		for _ in 0..iters {
			black_box(f());
		}
		start.elapsed()
	}

	let mut iters: u64 = 1;
	while time_batch(iters, &mut f) < BATCH_TARGET {
		iters *= 4;
	}

	let mut best = f64::INFINITY;
	let mut sum = 0.0;
	for _ in 0..SAMPLES {
		let ns = time_batch(iters, &mut f).as_nanos() as f64 / iters as f64;
		best = best.min(ns);
		sum += ns;
	}
	println!(
		"{name:<52} best {best:>11.1} ns/iter   mean {:>11.1} ns/iter   ({SAMPLES}x{iters} iters)",
		sum / f64::from(SAMPLES)
	);
}
