use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::mpsc;

use crate::ws::session::SessionEvent;

/// Outbound channel handle to a connected device's writer task.
pub type DeviceTx = mpsc::UnboundedSender<SessionEvent>;

/// Registry of connected devices: device_id -> outbound sender. Lets background
/// tasks (heartbeat, scheduler) proactively push messages/audio to a device.
#[derive(Default)]
pub struct SessionRegistry {
    devices: Mutex<HashMap<String, (String, DeviceTx)>>, // device_id -> (actor_id, tx)
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) a device's outbound sender on connect.
    pub fn register(&self, device_id: &str, actor_id: &str, tx: DeviceTx) {
        self.devices
            .lock()
            .unwrap()
            .insert(device_id.to_string(), (actor_id.to_string(), tx));
        tracing::info!(device_id, online = self.online_count(), "device registered");
    }

    /// Remove a device on disconnect (only if the stored sender is the same one).
    pub fn unregister(&self, device_id: &str) {
        self.devices.lock().unwrap().remove(device_id);
        tracing::info!(device_id, online = self.online_count(), "device unregistered");
    }

    /// True if the device currently has a live session.
    pub fn is_online(&self, device_id: &str) -> bool {
        self.devices.lock().unwrap().contains_key(device_id)
    }

    pub fn online_count(&self) -> usize {
        self.devices.lock().unwrap().len()
    }

    pub fn online_devices(&self) -> Vec<String> {
        self.devices.lock().unwrap().keys().cloned().collect()
    }

    /// Push an event to a device. Returns false if offline or the channel is closed.
    pub fn push(&self, device_id: &str, event: SessionEvent) -> bool {
        let guard = self.devices.lock().unwrap();
        match guard.get(device_id) {
            Some((_, tx)) => tx.send(event).is_ok(),
            None => false,
        }
    }

    /// device_ids of all online devices belonging to `actor_id`.
    pub fn devices_for_actor(&self, actor_id: &str) -> Vec<String> {
        self.devices
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, (aid, _))| aid == actor_id)
            .map(|(did, _)| did.clone())
            .collect()
    }
}
