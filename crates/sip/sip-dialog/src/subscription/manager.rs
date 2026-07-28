//! Subscription manager for handling SIP event subscriptions
//!
//! This module provides the SubscriptionManager that handles subscription
//! lifecycle, expiry ownership, and NOTIFY processing according to RFC 6665.

use dashmap::DashMap;
use rvoip_sip_core::{
    builder::SimpleResponseBuilder, HeaderName, Request, Response, StatusCode, TypedHeader,
};
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::task::AbortHandle;
use tracing::{debug, warn};

use super::event_package::EventPackage;
use crate::dialog::{
    Dialog, DialogId, DialogState, SubscriptionState, SubscriptionTerminationReason,
};
use crate::errors::{DialogError, DialogResult};
use crate::events::DialogEvent;

/// Build the shared `dialog_lookup` key for a subscription, including the
/// RFC 6665 §4.5.2 `Event: pkg;id=<sid>` disambiguator. A 4th segment
/// distinguishes subscription keys from the 3-tuple keys used by regular
/// INVITE-driven dialogs, so the two key namespaces never collide.
fn subscription_lookup_key(
    call_id: &str,
    tag_a: &str,
    tag_b: &str,
    event_id: Option<&str>,
) -> String {
    format!("{}:{}:{}:{}", call_id, tag_a, tag_b, event_id.unwrap_or(""))
}

const EXPIRY_TASK_COMPLETION_TIMEOUT: Duration = Duration::from_secs(1);

struct SubscriptionExpiryTask {
    generation: u64,
    claimed: AtomicBool,
    abort: AbortHandle,
    completion: watch::Receiver<bool>,
}

struct SubscriptionExpiryAdmission {
    accepting: bool,
}

struct SubscriptionExpiryRegistry {
    tasks: DashMap<DialogId, Arc<SubscriptionExpiryTask>>,
    operation_gate: Mutex<()>,
    admission: StdMutex<SubscriptionExpiryAdmission>,
    next_generation: AtomicU64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SubscriptionExpiryError {
    RegistryClosed,
    GenerationExhausted,
    CompletionTimeout { incomplete: usize },
    StartBarrierClosed,
}

impl fmt::Display for SubscriptionExpiryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegistryClosed => formatter.write_str("subscription expiry registry is closed"),
            Self::GenerationExhausted => {
                formatter.write_str("subscription expiry generation space is exhausted")
            }
            Self::CompletionTimeout { incomplete } => write!(
                formatter,
                "{incomplete} subscription expiry task(s) did not complete during drain"
            ),
            Self::StartBarrierClosed => {
                formatter.write_str("subscription expiry task start barrier closed")
            }
        }
    }
}

impl SubscriptionExpiryRegistry {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            tasks: DashMap::new(),
            operation_gate: Mutex::new(()),
            admission: StdMutex::new(SubscriptionExpiryAdmission { accepting: true }),
            next_generation: AtomicU64::new(1),
        })
    }

    fn len(&self) -> usize {
        self.tasks.len()
    }

    fn remove_exact(
        &self,
        dialog_id: &DialogId,
        generation: u64,
    ) -> Option<Arc<SubscriptionExpiryTask>> {
        self.tasks
            .remove_if(dialog_id, |_, current| current.generation == generation)
            .map(|(_, task)| task)
    }

    fn owns_claimed(&self, dialog_id: &DialogId, generation: u64) -> bool {
        self.tasks.get(dialog_id).is_some_and(|current| {
            current.generation == generation && current.claimed.load(Ordering::Acquire)
        })
    }

    async fn claim_exact(&self, dialog_id: &DialogId, generation: u64) -> bool {
        let _operation = self.operation_gate.lock().await;
        self.tasks.get(dialog_id).is_some_and(|current| {
            current.generation == generation
                && current
                    .claimed
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
        })
    }

    async fn wait_for_completion_until(
        task: &SubscriptionExpiryTask,
        deadline: tokio::time::Instant,
    ) -> bool {
        let mut completion = task.completion.clone();
        if *completion.borrow() {
            return true;
        }
        tokio::time::timeout_at(deadline, async {
            loop {
                if *completion.borrow() {
                    return true;
                }
                if completion.changed().await.is_err() {
                    return *completion.borrow();
                }
            }
        })
        .await
        .unwrap_or(false)
    }

    async fn cancel_record(
        &self,
        dialog_id: &DialogId,
        task: &Arc<SubscriptionExpiryTask>,
    ) -> Result<(), SubscriptionExpiryError> {
        // A task which already claimed its exact generation is allowed to
        // finish the one canonical termination. Aborting it could commit the
        // state update but cancel the corresponding event publication.
        if !task.claimed.load(Ordering::Acquire) {
            task.abort.abort();
        }
        let deadline = tokio::time::Instant::now() + EXPIRY_TASK_COMPLETION_TIMEOUT;
        if Self::wait_for_completion_until(task, deadline).await {
            self.remove_exact(dialog_id, task.generation);
            Ok(())
        } else {
            Err(SubscriptionExpiryError::CompletionTimeout { incomplete: 1 })
        }
    }

    async fn schedule(
        self: &Arc<Self>,
        owner: Weak<SubscriptionManagerInner>,
        dialog_id: DialogId,
        duration: Duration,
    ) -> Result<u64, SubscriptionExpiryError> {
        let _operation = self.operation_gate.lock().await;
        if !self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .accepting
        {
            return Err(SubscriptionExpiryError::RegistryClosed);
        }

        if let Some(current) = self
            .tasks
            .get(&dialog_id)
            .map(|entry| Arc::clone(entry.value()))
        {
            self.cancel_record(&dialog_id, &current).await?;
        }

        let generation = self
            .next_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .map_err(|_| SubscriptionExpiryError::GenerationExhausted)?;

        let (start_tx, start_rx) = oneshot::channel();
        let (completion_tx, completion_rx) = watch::channel(false);
        let completion = SubscriptionExpiryCompletion {
            registry: Arc::downgrade(self),
            dialog_id: dialog_id.clone(),
            generation,
            completion: Some(completion_tx),
        };
        let registry = Arc::downgrade(self);
        let task_dialog_id = dialog_id.clone();
        let handle = tokio::spawn(async move {
            let _completion = completion;
            if start_rx.await.is_err() {
                return;
            }
            tokio::time::sleep(duration).await;

            let Some(registry) = registry.upgrade() else {
                return;
            };
            if !registry.claim_exact(&task_dialog_id, generation).await {
                return;
            }
            let Some(inner) = owner.upgrade() else {
                return;
            };
            let manager = SubscriptionManager { inner };
            if let Err(error) = manager
                .terminate_subscription_owned(
                    &task_dialog_id,
                    Some(SubscriptionTerminationReason::Expired),
                    Some(generation),
                )
                .await
            {
                warn!(
                    dialog_id = %task_dialog_id,
                    generation,
                    error_class = error.diagnostic_class(),
                    "Subscription expiry termination failed"
                );
            }
        });

        let task = Arc::new(SubscriptionExpiryTask {
            generation,
            claimed: AtomicBool::new(false),
            abort: handle.abort_handle(),
            completion: completion_rx,
        });
        self.tasks.insert(dialog_id.clone(), Arc::clone(&task));

        if start_tx.send(()).is_err() {
            task.abort.abort();
            self.remove_exact(&dialog_id, generation);
            return Err(SubscriptionExpiryError::StartBarrierClosed);
        }

        Ok(generation)
    }

    async fn cancel_dialog(&self, dialog_id: &DialogId) -> Result<(), SubscriptionExpiryError> {
        let _operation = self.operation_gate.lock().await;
        if let Some(task) = self
            .tasks
            .get(dialog_id)
            .map(|entry| Arc::clone(entry.value()))
        {
            self.cancel_record(dialog_id, &task).await?;
        }
        Ok(())
    }

    async fn close_all(&self) -> Result<(), SubscriptionExpiryError> {
        let _operation = self.operation_gate.lock().await;
        self.admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .accepting = false;

        let records = self
            .tasks
            .iter()
            .map(|entry| (entry.key().clone(), Arc::clone(entry.value())))
            .collect::<Vec<_>>();
        for (_, task) in &records {
            if !task.claimed.load(Ordering::Acquire) {
                task.abort.abort();
            }
        }

        let deadline = tokio::time::Instant::now() + EXPIRY_TASK_COMPLETION_TIMEOUT;
        let mut incomplete = 0usize;
        for (dialog_id, task) in records {
            if Self::wait_for_completion_until(&task, deadline).await {
                self.remove_exact(&dialog_id, task.generation);
            } else {
                incomplete += 1;
            }
        }
        if incomplete == 0 {
            Ok(())
        } else {
            Err(SubscriptionExpiryError::CompletionTimeout { incomplete })
        }
    }

    fn abort_all_on_drop(&self) {
        self.admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .accepting = false;
        for task in self.tasks.iter() {
            task.abort.abort();
        }
    }
}

struct SubscriptionExpiryCompletion {
    registry: Weak<SubscriptionExpiryRegistry>,
    dialog_id: DialogId,
    generation: u64,
    completion: Option<watch::Sender<bool>>,
}

impl Drop for SubscriptionExpiryCompletion {
    fn drop(&mut self) {
        if let Some(completion) = self.completion.take() {
            let _ = completion.send(true);
        }
        if let Some(registry) = self.registry.upgrade() {
            registry.remove_exact(&self.dialog_id, self.generation);
        }
    }
}

struct SubscriptionManagerInner {
    dialogs: Arc<DashMap<DialogId, Dialog>>,
    dialog_lookup: Arc<DashMap<String, DialogId>>,
    expiry_tasks: Arc<SubscriptionExpiryRegistry>,
    event_packages: Arc<DashMap<String, Box<dyn EventPackage>>>,
    event_tx: mpsc::Sender<DialogEvent>,
}

impl Drop for SubscriptionManagerInner {
    fn drop(&mut self) {
        self.expiry_tasks.abort_all_on_drop();
    }
}

/// Manages SIP event subscriptions
pub struct SubscriptionManager {
    inner: Arc<SubscriptionManagerInner>,
}

impl SubscriptionManager {
    /// Create a new subscription manager with shared dialog stores
    pub fn new(
        dialogs: Arc<DashMap<DialogId, Dialog>>,
        dialog_lookup: Arc<DashMap<String, DialogId>>,
        event_tx: mpsc::Sender<DialogEvent>,
    ) -> Self {
        let mut manager = Self {
            inner: Arc::new(SubscriptionManagerInner {
                dialogs,
                dialog_lookup,
                expiry_tasks: SubscriptionExpiryRegistry::new(),
                event_packages: Arc::new(DashMap::new()),
                event_tx,
            }),
        };

        // Register default event packages
        manager.register_default_packages();

        manager
    }

    /// Register default event packages
    fn register_default_packages(&mut self) {
        use super::event_package::{
            DialogPackage, MessageSummaryPackage, PresencePackage, ReferPackage,
        };

        self.register_event_package(Box::new(PresencePackage));
        self.register_event_package(Box::new(DialogPackage));
        self.register_event_package(Box::new(MessageSummaryPackage));
        self.register_event_package(Box::new(ReferPackage));
    }

    /// Register an event package
    pub fn register_event_package(&mut self, package: Box<dyn EventPackage>) {
        let name = package.name().to_string();
        self.inner.event_packages.insert(name, package);
    }

    /// Handle incoming SUBSCRIBE request
    pub async fn handle_subscribe(
        &self,
        request: Request,
        _source: SocketAddr,
        _local_addr: SocketAddr,
    ) -> DialogResult<(Response, Option<DialogId>)> {
        // Extract Event header
        let event = request
            .header(&HeaderName::Event)
            .and_then(|h| match h {
                TypedHeader::Event(e) => Some(e),
                _ => None,
            })
            .ok_or_else(|| DialogError::protocol_error("SUBSCRIBE requires Event header"))?;

        let event_package = event.event_type.to_string();

        // Check if event package is supported
        if !self.inner.event_packages.contains_key(&event_package) {
            // Return 489 Bad Event
            // Build 489 Bad Event response
            let supported_events = self.get_supported_events();
            let event_refs: Vec<&str> = supported_events.iter().map(|s| s.as_str()).collect();
            let response = self
                .build_response_from_request(StatusCode::BadEvent, None, &request)
                .allow_events(&event_refs)
                .build();
            return Ok((response, None));
        }

        // Get the event package handler
        let package = self
            .inner
            .event_packages
            .get(&event_package)
            .ok_or_else(|| DialogError::protocol_error("Event package not found"))?;

        // Extract Expires header
        let expires = request
            .header(&HeaderName::Expires)
            .and_then(|h| match h {
                TypedHeader::Expires(e) => Some(e.0),
                _ => None,
            })
            .unwrap_or(package.default_expires().as_secs() as u32);

        // Validate expires against package limits
        let min_expires = package.min_expires().as_secs() as u32;
        let max_expires = package.max_expires().as_secs() as u32;

        if expires < min_expires && expires != 0 {
            // Return 423 Interval Too Brief
            // Return 423 Interval Too Brief
            let response = self
                .build_response_from_request(StatusCode::IntervalTooBrief, None, &request)
                .min_expires(min_expires)
                .build();
            return Ok((response, None));
        }

        let adjusted_expires = if expires > max_expires {
            max_expires
        } else {
            expires
        };
        drop(package);

        // Create subscription dialog
        let dialog_id = DialogId::new();
        let local_tag = format!("{:08x}", rand::random::<u32>());

        // Extract dialog information from request
        let call_id = request
            .call_id()
            .map(|c| c.value().to_string())
            .unwrap_or_else(|| format!("sub-{}", uuid::Uuid::new_v4()));

        let local_uri = request
            .to()
            .map(|t| t.uri.clone())
            .ok_or_else(|| DialogError::protocol_error("Missing To header"))?;

        let remote_uri = request
            .from()
            .map(|f| f.uri.clone())
            .ok_or_else(|| DialogError::protocol_error("Missing From header"))?;

        let remote_tag = request.from().and_then(|f| f.tag().map(|t| t.to_string()));

        // Create the dialog
        let mut dialog = Dialog::new(
            call_id.clone(),
            local_uri,
            remote_uri,
            Some(local_tag.clone()),
            remote_tag,
            false, // Not initiator for incoming SUBSCRIBE
        );
        dialog.id = dialog_id.clone();
        dialog.state = DialogState::Early; // Early until we send 200 OK
        dialog.remote_cseq = request.cseq().map(|c| c.seq).unwrap_or(1);

        // Set subscription-specific fields
        dialog.subscription_state = Some(if adjusted_expires > 0 {
            SubscriptionState::Pending
        } else {
            SubscriptionState::Terminated {
                reason: Some(SubscriptionTerminationReason::ClientRequested),
            }
        });
        dialog.event_package = Some(event_package.clone());
        dialog.event_id = event.id.clone();

        // Store the dialog
        self.inner.dialogs.insert(dialog_id.clone(), dialog.clone());

        // Add to lookup table. The key includes the subscription's event id
        // parameter so multiple subscriptions on the same dialog (RFC 6665
        // §4.5.2) don't clobber each other in the shared lookup map.
        let lookup_key = subscription_lookup_key(
            &dialog.call_id,
            dialog.local_tag.as_deref().unwrap_or(""),
            dialog.remote_tag.as_deref().unwrap_or(""),
            dialog.event_id.as_deref(),
        );
        self.inner
            .dialog_lookup
            .insert(lookup_key.clone(), dialog_id.clone());

        // Start the exact expiry owner if the subscription is active.
        if adjusted_expires > 0 {
            if let Err(error) = self
                .start_expiry_task(
                    dialog_id.clone(),
                    Duration::from_secs(adjusted_expires as u64),
                )
                .await
            {
                self.inner.dialogs.remove(&dialog_id);
                self.inner
                    .dialog_lookup
                    .remove_if(&lookup_key, |_, mapped| mapped == &dialog_id);
                return Err(error);
            }
        }

        // Build 200 OK response
        // Build 200 OK response
        let mut response = self
            .build_response_from_request(StatusCode::Ok, None, &request)
            .expires(adjusted_expires);

        // Add To tag for dialog creation
        if let Some(to) = request.to() {
            response = response.to(
                to.display_name.as_deref().unwrap_or(""),
                &to.uri.to_string(),
                Some(&local_tag),
            );
        }

        let response = response.build();

        // This legacy DialogEvent channel is observational only. The caller
        // owns the SUBSCRIBE response and any initial NOTIFY policy; a full or
        // absent observer must never delay the wire response.
        let _ = self
            .inner
            .event_tx
            .try_send(DialogEvent::SubscriptionCreated {
                dialog_id: dialog_id.clone(),
                event_package,
                expires: Duration::from_secs(adjusted_expires as u64),
            });

        Ok((response, Some(dialog_id)))
    }

    /// Handle incoming NOTIFY request
    pub async fn handle_notify(
        &self,
        request: Request,
        _source: SocketAddr,
    ) -> DialogResult<Response> {
        // Extract Subscription-State header
        let subscription_state = request
            .header(&HeaderName::SubscriptionState)
            .and_then(|h| match h {
                TypedHeader::SubscriptionState(s) => Some(s),
                _ => None,
            })
            .ok_or_else(|| {
                DialogError::protocol_error("NOTIFY requires Subscription-State header")
            })?;

        // Find subscription by Call-ID, tags, and Event header `id` parameter.
        // Per RFC 6665 §4.5.2, multiple subscriptions can share one dialog;
        // the `Event: pkg;id=<sid>` parameter disambiguates them.
        let call_id = request
            .call_id()
            .map(|c| c.value().to_string())
            .ok_or_else(|| DialogError::protocol_error("Missing Call-ID"))?;

        let to_tag = request.to().and_then(|t| t.tag().map(|s| s.to_string()));

        let from_tag = request.from().and_then(|f| f.tag().map(|s| s.to_string()));

        let event_id = request.header(&HeaderName::Event).and_then(|h| match h {
            TypedHeader::Event(e) => e.id.clone(),
            _ => None,
        });

        let lookup_key = subscription_lookup_key(
            &call_id,
            to_tag.as_deref().unwrap_or(""),
            from_tag.as_deref().unwrap_or(""),
            event_id.as_deref(),
        );

        if let Some(dialog_id_entry) = self.inner.dialog_lookup.get(&lookup_key) {
            let dialog_id = dialog_id_entry.value().clone();
            drop(dialog_id_entry);

            if let Some(mut dialog) = self.inner.dialogs.get_mut(&dialog_id) {
                // Update subscription state
                let new_state =
                    SubscriptionState::from_header_value(&subscription_state.to_string());
                dialog.subscription_state = Some(new_state.clone());
                let terminated = new_state.is_terminated();
                drop(dialog);

                // The DialogManager's typed causal sink owns session
                // processing. Keep this public compatibility channel as a
                // best-effort observation only.
                let _ = self.inner.event_tx.try_send(DialogEvent::NotifyReceived {
                    dialog_id: dialog_id.clone(),
                    state: new_state.clone(),
                    body: if !request.body().is_empty() {
                        Some(request.body().to_vec())
                    } else {
                        None
                    },
                });

                // If subscription is terminated, clean up
                if terminated {
                    self.cleanup_subscription(&dialog_id).await?;
                }
            }
        }

        // Always respond 200 OK to NOTIFY (RFC 6665)
        // Always respond 200 OK to NOTIFY (RFC 6665)
        let response = self
            .build_response_from_request(StatusCode::Ok, None, &request)
            .build();

        Ok(response)
    }

    /// Mark subscription as active after initial NOTIFY is sent
    pub async fn activate_subscription(&self, dialog_id: &DialogId) -> DialogResult<()> {
        if let Some(mut dialog) = self.inner.dialogs.get_mut(dialog_id) {
            if let Some(SubscriptionState::Pending) = &dialog.subscription_state {
                dialog.subscription_state = Some(SubscriptionState::Active {
                    remaining_duration: Duration::from_secs(3600),
                    original_duration: Duration::from_secs(3600),
                });
                dialog.state = DialogState::Confirmed;
                debug!("Subscription {} activated", dialog_id);
            }
        }
        Ok(())
    }

    /// Install the one exact-dialog expiry owner for this generation.
    async fn start_expiry_task(
        &self,
        dialog_id: DialogId,
        duration: Duration,
    ) -> DialogResult<u64> {
        self.inner
            .expiry_tasks
            .schedule(Arc::downgrade(&self.inner), dialog_id, duration)
            .await
            .map_err(Self::expiry_error)
    }

    /// Clean up a terminated subscription
    async fn cleanup_subscription(&self, dialog_id: &DialogId) -> DialogResult<()> {
        self.inner
            .expiry_tasks
            .cancel_dialog(dialog_id)
            .await
            .map_err(Self::expiry_error)?;
        self.mark_subscription_terminated(dialog_id);
        Ok(())
    }

    fn mark_subscription_terminated(&self, dialog_id: &DialogId) {
        if let Some(mut dialog) = self.inner.dialogs.get_mut(dialog_id) {
            dialog.state = DialogState::Terminated;
        }
        debug!("Cleaned up subscription {}", dialog_id);
    }

    fn expiry_error(error: SubscriptionExpiryError) -> DialogError {
        DialogError::internal_error(
            &format!("Subscription expiry lifecycle failed: {error}"),
            None,
        )
    }

    /// Close admission and observe every expiry task's completion. This is
    /// called by `DialogManager::stop` before the dialog store is cleared.
    pub(crate) async fn close_expiry_tasks(&self) -> DialogResult<()> {
        self.inner
            .expiry_tasks
            .close_all()
            .await
            .map_err(Self::expiry_error)
    }

    #[cfg(test)]
    fn expiry_task_count(&self) -> usize {
        self.inner.expiry_tasks.len()
    }

    /// Build a response from a request, copying necessary headers
    fn build_response_from_request(
        &self,
        status: StatusCode,
        reason: Option<&str>,
        request: &Request,
    ) -> SimpleResponseBuilder {
        let mut builder = SimpleResponseBuilder::new(status, reason);

        // Copy required headers
        if let Some(from) = request.from() {
            builder = builder.from(
                from.display_name.as_deref().unwrap_or(""),
                &from.uri.to_string(),
                from.tag().as_deref(),
            );
        }

        if let Some(to) = request.to() {
            builder = builder.to(
                to.display_name.as_deref().unwrap_or(""),
                &to.uri.to_string(),
                to.tag().as_deref(),
            );
        }

        if let Some(call_id) = request.call_id() {
            builder = builder.call_id(&call_id.value());
        }

        if let Some(cseq) = request.cseq() {
            builder = builder.cseq(cseq.seq, cseq.method.clone());
        }

        if let Some(via) = request.first_via() {
            if let Some(first_via_header) = via.0.first() {
                let host = format!(
                    "{}:{}",
                    first_via_header.sent_by_host,
                    first_via_header.sent_by_port.unwrap_or(5060)
                );
                let transport = &first_via_header.sent_protocol.transport;
                builder = builder.via(&host, transport, first_via_header.branch().as_deref());
            }
        }

        builder
    }

    /// Get list of supported event packages
    fn get_supported_events(&self) -> Vec<String> {
        self.inner
            .event_packages
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Terminate a subscription
    pub async fn terminate_subscription(
        &self,
        dialog_id: &DialogId,
        reason: Option<SubscriptionTerminationReason>,
    ) -> DialogResult<()> {
        self.terminate_subscription_owned(dialog_id, reason, None)
            .await
    }

    async fn terminate_subscription_owned(
        &self,
        dialog_id: &DialogId,
        reason: Option<SubscriptionTerminationReason>,
        expiry_generation: Option<u64>,
    ) -> DialogResult<()> {
        if let Some(generation) = expiry_generation {
            if !self.inner.expiry_tasks.owns_claimed(dialog_id, generation) {
                return Ok(());
            }
        } else {
            // An explicit termination owns the result only after the previous
            // expiry generation is either cancelled or has completed.
            self.inner
                .expiry_tasks
                .cancel_dialog(dialog_id)
                .await
                .map_err(Self::expiry_error)?;
        }

        let already_terminated = self
            .inner
            .dialogs
            .get(dialog_id)
            .and_then(|dialog| dialog.subscription_state.clone())
            .is_some_and(|state| state.is_terminated());
        if already_terminated {
            self.mark_subscription_terminated(dialog_id);
            return Ok(());
        }

        let Some(mut dialog) = self.inner.dialogs.get_mut(dialog_id) else {
            return Ok(());
        };
        dialog.subscription_state = Some(SubscriptionState::Terminated {
            reason: reason.clone(),
        });
        drop(dialog);

        // Report the committed termination without putting observer
        // backpressure on expiry ownership or shutdown drain.
        let _ = self
            .inner
            .event_tx
            .try_send(DialogEvent::SubscriptionTerminated {
                dialog_id: dialog_id.clone(),
                reason: reason.map(|value| value.to_string()),
            });

        self.mark_subscription_terminated(dialog_id);
        Ok(())
    }
}

// Debug implementation for SubscriptionManager
impl std::fmt::Debug for SubscriptionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionManager")
            .field("dialogs", &self.inner.dialogs.len())
            .field("expiry_tasks", &self.inner.expiry_tasks.len())
            .field("event_packages", &self.inner.event_packages.len())
            .finish()
    }
}

// Clone implementation for SubscriptionManager
impl Clone for SubscriptionManager {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::types::Uri;

    fn manager_with_subscription() -> (
        SubscriptionManager,
        Arc<DashMap<DialogId, Dialog>>,
        mpsc::Receiver<DialogEvent>,
        DialogId,
    ) {
        let dialogs = Arc::new(DashMap::new());
        let lookup = Arc::new(DashMap::new());
        let (event_tx, event_rx) = mpsc::channel(8);
        let manager = SubscriptionManager::new(Arc::clone(&dialogs), lookup, event_tx);
        let local_uri: Uri = "sip:subscriber@example.com".parse().unwrap();
        let remote_uri: Uri = "sip:notifier@example.com".parse().unwrap();
        let dialog_id = DialogId::new();
        let mut dialog = Dialog::new(
            "subscription-expiry-test".to_string(),
            local_uri,
            remote_uri,
            Some("local-tag".to_string()),
            Some("remote-tag".to_string()),
            true,
        );
        dialog.id = dialog_id.clone();
        dialog.event_package = Some("presence".to_string());
        dialog.subscription_state = Some(SubscriptionState::Pending);
        dialogs.insert(dialog_id.clone(), dialog);
        (manager, dialogs, event_rx, dialog_id)
    }

    async fn wait_for_no_expiry_tasks(manager: &SubscriptionManager) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while manager.expiry_task_count() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("expiry task record must be released");
    }

    #[tokio::test]
    async fn exact_expiry_terminates_once_and_releases_task_record() {
        let (manager, dialogs, mut events, dialog_id) = manager_with_subscription();
        manager
            .start_expiry_task(dialog_id.clone(), Duration::from_millis(20))
            .await
            .unwrap();

        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("expiry event timeout")
            .expect("expiry event channel closed");
        match event {
            DialogEvent::SubscriptionTerminated {
                dialog_id: expired_dialog,
                reason,
            } => {
                assert_eq!(expired_dialog, dialog_id);
                assert_eq!(reason.as_deref(), Some("expired"));
            }
            other => panic!("expected subscription termination, got {other:?}"),
        }

        wait_for_no_expiry_tasks(&manager).await;
        let dialog = dialogs.get(&dialog_id).unwrap();
        assert_eq!(dialog.state, DialogState::Terminated);
        assert_eq!(
            dialog.subscription_state,
            Some(SubscriptionState::Terminated {
                reason: Some(SubscriptionTerminationReason::Expired),
            })
        );
        drop(dialog);
        assert!(
            tokio::time::timeout(Duration::from_millis(60), events.recv())
                .await
                .is_err(),
            "one expiry generation must emit exactly one termination"
        );
        manager.close_expiry_tasks().await.unwrap();
    }

    #[tokio::test]
    async fn stale_generation_cannot_remove_or_fire_replacement() {
        let (manager, _dialogs, mut events, dialog_id) = manager_with_subscription();
        let stale_generation = manager
            .start_expiry_task(dialog_id.clone(), Duration::from_secs(60))
            .await
            .unwrap();
        let replacement_generation = manager
            .start_expiry_task(dialog_id.clone(), Duration::from_millis(20))
            .await
            .unwrap();
        assert_ne!(stale_generation, replacement_generation);

        // This is the same exact removal a late completion guard performs.
        // It must not erase the newer dialog generation.
        assert!(manager
            .inner
            .expiry_tasks
            .remove_exact(&dialog_id, stale_generation)
            .is_none());
        assert_eq!(manager.expiry_task_count(), 1);

        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("replacement expiry event timeout")
            .expect("replacement expiry event channel closed");
        assert!(matches!(
            event,
            DialogEvent::SubscriptionTerminated { reason, .. }
                if reason.as_deref() == Some("expired")
        ));
        wait_for_no_expiry_tasks(&manager).await;
        manager.close_expiry_tasks().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_closes_admission_and_drains_long_expiry() {
        let (manager, _dialogs, mut events, dialog_id) = manager_with_subscription();
        manager
            .start_expiry_task(dialog_id.clone(), Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(manager.expiry_task_count(), 1);

        manager.close_expiry_tasks().await.unwrap();
        assert_eq!(manager.expiry_task_count(), 0);
        assert!(events.try_recv().is_err());
        let error = manager
            .start_expiry_task(dialog_id, Duration::from_secs(60))
            .await
            .expect_err("closed registry must reject new expiry work");
        assert_eq!(error.diagnostic_class(), "internal");
    }

    #[tokio::test]
    async fn saturated_observer_cannot_delay_expiry_or_shutdown_drain() {
        let (manager, dialogs, _events, dialog_id) = manager_with_subscription();
        for _ in 0..8 {
            manager
                .inner
                .event_tx
                .try_send(DialogEvent::ShutdownReady)
                .expect("fill observational channel");
        }

        manager
            .start_expiry_task(dialog_id.clone(), Duration::from_millis(20))
            .await
            .expect("start exact expiry with saturated observer");
        wait_for_no_expiry_tasks(&manager).await;
        assert_eq!(
            dialogs.get(&dialog_id).expect("subscription dialog").state,
            DialogState::Terminated,
            "observer saturation must not suppress the committed expiry"
        );
        manager
            .close_expiry_tasks()
            .await
            .expect("observer saturation must not block drain");
    }
}
