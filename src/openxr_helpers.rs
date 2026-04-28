use std::mem::MaybeUninit;

use rustix::fs::Timespec;
use stardust_xr_protocol::types::Timestamp;

pub trait ConvertTimespec {
	fn timestamp_to_xr(&self, time: Timestamp) -> Option<openxr::Time>;
	fn xr_to_timestamp(&self, time: openxr::Time) -> Option<Timestamp>;
}
impl ConvertTimespec for openxr::Instance {
	fn timestamp_to_xr(&self, time: Timestamp) -> Option<openxr::Time> {
		let time_ext = self.exts().khr_convert_timespec_time?;
		unsafe {
			let mut out = MaybeUninit::uninit();
			// this timespec struct has the same abi as the libc one, which is what OpenXR expects
			let timespec = Timespec {
				tv_sec: time.seconds,
				tv_nsec: time.nanoseconds,
			};
			let result = (time_ext.convert_timespec_time_to_time)(
				self.as_raw(),
				(&raw const timespec).cast(),
				out.as_mut_ptr(),
			);
			if result == openxr::sys::Result::SUCCESS {
				let v = out.assume_init();
				Some(v)
			} else {
				None
			}
		}
	}

	fn xr_to_timestamp(&self, time: openxr::Time) -> Option<Timestamp> {
		let time_ext = self.exts().khr_convert_timespec_time?;
		unsafe {
			let mut out = MaybeUninit::uninit();
			let result =
				(time_ext.convert_time_to_timespec_time)(self.as_raw(), time, out.as_mut_ptr());
			if result == openxr::sys::Result::SUCCESS {
				let v = out.assume_init();
				Some(Timestamp {
					seconds: v.tv_sec,
					nanoseconds: v.tv_nsec,
				})
			} else {
				None
			}
		}
	}
}
