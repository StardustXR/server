use crate::error::{Result, ServerError};
use serde::Serialize;
use stardust_xr_wire::{flex::serialize, messenger::MethodResponse, scenegraph::ScenegraphError};

pub struct MethodResponseSender(pub(crate) MethodResponse);
impl MethodResponseSender {
	pub fn send_err(self, error: ScenegraphError) {
		self.0.send(Err(error));
	}
	pub fn send<T: Serialize>(self, result: Result<T, ServerError>) {
		let data = match result {
			Ok(d) => d,
			Err(e) => {
				self.0.send(Err(ScenegraphError::MemberError {
					error: e.to_string(),
				}));
				return;
			}
		};
		let Ok((serialized, fds)) = stardust_xr_wire::flex::serialize(data) else {
			self.0.send(Err(ScenegraphError::MemberError {
				error: "Internal: Failed to serialize".to_string(),
			}));
			return;
		};
		self.0.send(Ok((&serialized, fds)));
	}
	pub fn wrap<T: Serialize, F: FnOnce() -> Result<T>>(self, f: F) {
		self.send(f())
	}
	pub fn wrap_async<T: Serialize>(self, f: impl Future<Output = Result<T>> + Send + 'static) {
		tokio::task::spawn(async move {
			let value = match f.await {
				Ok(d) => d,
				Err(e) => {
					self.0.send(Err(ScenegraphError::MemberError {
						error: e.to_string(),
					}));
					return;
				}
			};
			let Ok((serialized, fds)) = serialize(value) else {
				self.0.send(Err(ScenegraphError::MemberError {
					error: "Internal: Failed to serialize".to_string(),
				}));
				return;
			};
			self.0.send(Ok((&serialized, fds)));
		});
	}
}
impl From<MethodResponse> for MethodResponseSender {
	fn from(response: MethodResponse) -> Self {
		Self(response)
	}
}
impl std::fmt::Debug for MethodResponseSender {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("TypedMethodResponse").finish()
	}
}
