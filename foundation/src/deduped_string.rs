use std::{
	collections::HashMap,
	sync::{Arc, LazyLock, Weak},
};

use tokio::sync::RwLock;

static STRING_TO_DEDUP: LazyLock<RwLock<HashMap<String, Weak<DedupedStr>>>> =
	LazyLock::new(|| RwLock::new(HashMap::new()));
#[derive(Debug, Hash, PartialEq, Eq)]
pub struct DedupedStr(String);
impl DedupedStr {
	pub async fn get(str: String) -> Arc<DedupedStr> {
		if let Some(v) = STRING_TO_DEDUP
			.read()
			.await
			.get(&str)
			.and_then(|v| v.upgrade())
		{
			return v;
		}

		let v = Arc::from(DedupedStr(str.clone()));
		STRING_TO_DEDUP
			.write()
			.await
			.insert(str, Arc::downgrade(&v));
		v
	}
	pub fn get_string(&self) -> &String {
		&self.0
	}
}
impl Drop for DedupedStr {
	fn drop(&mut self) {
		// could probably remove this clone, but does it matter?
		let v = self.0.clone();
		tokio::spawn(async move { STRING_TO_DEDUP.write().await.remove(&v) });
	}
}
