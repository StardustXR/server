use rustix::time::ClockId;
use stardust_xr_protocol::types::Timestamp;

pub trait TimestampExt {
	fn now() -> Self;
}
impl TimestampExt for Timestamp {
	fn now() -> Self {
		let time = rustix::time::clock_gettime(ClockId::Monotonic);
		Timestamp {
			seconds: time.tv_sec,
			nanoseconds: time.tv_nsec,
		}
	}
}
