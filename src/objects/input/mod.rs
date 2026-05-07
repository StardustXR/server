pub mod mouse_pointer;
// pub mod oxr_controller;
pub mod oxr_hand;

use crate::nodes::{
    ProxyExt as _,
    fields::FieldRef,
    spatial::SpatialRef,
};
use binderbinder::binder_object::{BinderObjectRef, ToBinderObjectOrRef};
use stardust_xr_protocol::{
    field::FieldRef as FieldRefProxy,
    query::{QueriedInterface, QueryableObjectRef},
    spatial::SpatialRef as SpatialRefProxy,
    suis::InputHandler,
};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};
use tokio::sync::RwLock;

pub struct CachedHandler<V: Send + Sync + 'static> {
    pub handler: InputHandler,
    pub spatial: BinderObjectRef<SpatialRef>,
    pub field: BinderObjectRef<FieldRef>,
    pub value: V,
}

impl<V: Send + Sync + 'static> fmt::Debug for CachedHandler<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachedHandler").finish_non_exhaustive()
    }
}

pub struct QueryHandler<V: Send + Sync + 'static> {
    pub handlers: Arc<RwLock<HashMap<QueryableObjectRef, CachedHandler<V>>>>,
}

impl<V: Send + Sync + 'static> fmt::Debug for QueryHandler<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueryHandler").finish_non_exhaustive()
    }
}

impl<V: Send + Sync + 'static> QueryHandler<V> {
    pub fn new() -> (Self, Arc<RwLock<HashMap<QueryableObjectRef, CachedHandler<V>>>>) {
        let handlers = Arc::new(RwLock::new(HashMap::new()));
        (Self { handlers: handlers.clone() }, handlers)
    }

    pub async fn on_entered(
        &self,
        obj: QueryableObjectRef,
        field: FieldRefProxy,
        spatial: SpatialRefProxy,
        interfaces: Vec<QueriedInterface>,
        value: V,
    ) {
        let Some(interface) = interfaces.first() else {
            return;
        };
        if interface.interface_id != "org.stardustxr.SUIS.Handler" {
            return;
        }
        let Some(spatial) = spatial.owned() else { return };
        let Some(field) = field.owned() else { return };
        let handler = InputHandler::from_object_or_ref(interface.interface.clone());
        self.handlers.write().await.insert(obj, CachedHandler {
            handler,
            spatial,
            field,
            value,
        });
    }

    pub async fn on_value_changed(&self, obj: &QueryableObjectRef, new_value: V) {
        if let Some(entry) = self.handlers.write().await.get_mut(obj) {
            entry.value = new_value;
        }
    }

    pub async fn on_left(&self, obj: &QueryableObjectRef) {
        self.handlers.write().await.remove(obj);
    }
}

#[derive(Default)]
pub struct HandlerTracker {
    active: HashSet<InputHandler>,
}

impl HandlerTracker {
    pub fn update(
        &mut self,
        new: HashSet<InputHandler>,
    ) -> (HashSet<InputHandler>, HashSet<InputHandler>) {
        let added = new.difference(&self.active).cloned().collect();
        let removed = self.active.difference(&new).cloned().collect();
        self.active = new;
        (added, removed)
    }
}

pub struct InputMethodBase<V: Send + Sync + 'static> {
    pub capture: RwLock<Option<InputHandler>>,
    pub capture_requests: RwLock<HashSet<InputHandler>>,
    pub handlers: Arc<RwLock<HashMap<QueryableObjectRef, CachedHandler<V>>>>,
}

impl<V: Send + Sync + 'static> fmt::Debug for InputMethodBase<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputMethodBase").finish_non_exhaustive()
    }
}

impl<V: Send + Sync + 'static> InputMethodBase<V> {
    pub fn new(handlers: Arc<RwLock<HashMap<QueryableObjectRef, CachedHandler<V>>>>) -> Self {
        Self {
            capture: RwLock::new(None),
            capture_requests: RwLock::new(HashSet::new()),
            handlers,
        }
    }

    pub async fn request_capture(&self, handler: InputHandler) {
        if self
            .handlers
            .read()
            .await
            .values()
            .any(|e| e.handler == handler)
        {
            self.capture_requests.write().await.insert(handler);
        }
    }

    pub async fn release_capture(&self, handler: &InputHandler) {
        self.capture_requests.write().await.remove(handler);
        if self.capture.read().await.as_ref() == Some(handler) {
            self.capture.write().await.take();
        }
    }

    pub async fn maybe_promote_capture(&self, handler: &InputHandler) {
        if self.capture_requests.read().await.contains(handler)
            && self.capture.read().await.is_none()
        {
            self.capture.write().await.replace(handler.clone());
        }
    }
}
