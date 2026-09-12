///////////////////////////////////////////////////////////////////////////////
//
//  Copyright 2018-2026 Robonomics Network <research@robonomics.network>
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
//
///////////////////////////////////////////////////////////////////////////////
//! Coordinated graceful-shutdown primitive.
//!
//! A single [`ShutdownController`] drives shutdown for every module. Components
//! hold a [`ShutdownSignal`] and await [`ShutdownSignal::recv`] to learn when it
//! is time to drain and stop. This is intentionally tiny (a [`tokio::sync::watch`]
//! channel) so it can be cloned freely across tasks on constrained hardware.

use tokio::sync::watch;

/// Owner side of the shutdown mechanism.
///
/// Dropping the controller also triggers shutdown, guarding against tasks that
/// would otherwise wait forever if the controller is lost.
#[derive(Debug)]
pub struct ShutdownController {
    tx: watch::Sender<bool>,
}

impl ShutdownController {
    /// Creates a new controller in the "running" state.
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self { tx }
    }

    /// Returns a fresh [`ShutdownSignal`] that observers can await.
    pub fn subscribe(&self) -> ShutdownSignal {
        ShutdownSignal {
            rx: self.tx.subscribe(),
        }
    }

    /// Signals all subscribers to begin shutting down.
    pub fn trigger(&self) {
        // Ignore send errors: they only occur when there are no receivers.
        let _ = self.tx.send(true);
    }
}

impl Default for ShutdownController {
    fn default() -> Self {
        Self::new()
    }
}

/// Observer side of the shutdown mechanism, cheaply cloneable.
#[derive(Clone, Debug)]
pub struct ShutdownSignal {
    rx: watch::Receiver<bool>,
}

impl ShutdownSignal {
    /// Returns `true` if shutdown has already been requested.
    pub fn is_triggered(&self) -> bool {
        *self.rx.borrow()
    }

    /// Resolves once shutdown has been requested.
    ///
    /// Returns immediately if shutdown was already triggered. Also resolves if
    /// the [`ShutdownController`] was dropped, so callers never wait forever.
    pub async fn recv(&mut self) {
        if *self.rx.borrow() {
            return;
        }
        // `changed` only errors when the sender is dropped, which we treat as a
        // shutdown request.
        let _ = self.rx.changed().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn trigger_wakes_subscribers() {
        let controller = ShutdownController::new();
        let mut signal = controller.subscribe();
        assert!(!signal.is_triggered());
        controller.trigger();
        signal.recv().await;
        assert!(signal.is_triggered());
    }

    #[tokio::test]
    async fn drop_controller_wakes_subscribers() {
        let controller = ShutdownController::new();
        let mut signal = controller.subscribe();
        drop(controller);
        // Should resolve rather than hang.
        signal.recv().await;
    }
}
