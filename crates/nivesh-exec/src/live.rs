//! The live route: the **only** code that calls [`BrokerExecution`].
//!
//! The gateway mints a [`LiveDispatch`] when an armed strategy's intent
//! passes every check. The dispatch holds the approval and its client
//! tag; it cannot be built outside this crate, cloned or sent twice
//! ([`LiveDispatch::send`] takes it by value). The gateway runs
//! synchronously under the harness's lock, so the harness awaits `send`
//! after releasing it.
//!
//! Sending fails closed:
//!
//! - the routed order is already on the audit log before a dispatch
//!   exists; the broker's answer is audited after;
//! - an approval that expired while waiting is never sent;
//! - placement is tried exactly once. An ambiguous outcome (a timeout, an
//!   upstream error) is tracked in the [`LiveBook`] with no broker id and
//!   reported as [`GatewayError::AmbiguousPlacement`]: the harness pauses
//!   the strategy and reconciles by the tag. It is never placed again.

use std::sync::{Arc, Mutex, MutexGuard};

use nivesh_broker::{BrokerError, BrokerExecution, OrderAck, OrderStatus, OrderTag};
use nivesh_core::InstrumentId;
use nivesh_risk::ApprovedOrder;

use crate::reconcile::{LiveBook, TrackedOrder};
use crate::{AuditEvent, AuditSink, GatewayError};

/// A broker that can trade, with the book of what Nivesh sent it.
pub struct LiveRoute {
    broker: Arc<dyn BrokerExecution>,
    audit: Arc<dyn AuditSink>,
    book: Mutex<LiveBook>,
}

impl LiveRoute {
    pub fn new(broker: Arc<dyn BrokerExecution>, audit: Arc<dyn AuditSink>) -> Self {
        Self {
            broker,
            audit,
            book: Mutex::new(LiveBook::default()),
        }
    }

    /// What Nivesh believes it sent, for reconciliation.
    pub fn book(&self) -> LiveBook {
        self.lock().clone()
    }

    /// Replaces the book (after a reconciliation adopted the broker's view,
    /// or at startup from the saved book).
    pub fn set_book(&self, book: LiveBook) {
        *self.lock() = book;
    }

    /// Cancels every open Nivesh-tagged order at the broker: the kill
    /// switch's second half (the first, blocking new orders, never waits on
    /// the network). The broker's own book is read, so orders from an
    /// ambiguous placement or another session are found too. Each request
    /// is audited (best-effort) with its outcome, and nothing is retried.
    /// Returns the tags cancelled and those that failed, with why.
    pub async fn cancel_open(&self, now_ms: i64) -> Result<CancelReport, BrokerError> {
        let open: Vec<_> = self
            .broker
            .orders()
            .await?
            .into_iter()
            .filter(|o| o.status.is_live() && o.tag.as_deref().is_some_and(OrderTag::is_nivesh))
            .collect();
        let mut report = CancelReport::default();
        for order in open {
            let tag = order.tag.clone().unwrap_or_default();
            let result = self.broker.cancel(&order.id).await;
            let error = result.as_ref().err().map(ToString::to_string);
            let _ = self.audit.record(
                now_ms,
                &AuditEvent::LiveCancel {
                    tag: tag.clone(),
                    broker_id: order.id.to_string(),
                    error: error.clone(),
                },
            );
            match error {
                None => {
                    if let Some(t) = self
                        .lock()
                        .orders
                        .iter_mut()
                        .find(|t| t.tag.as_str() == tag)
                    {
                        t.status = Some(OrderStatus::Cancelled);
                    }
                    report.cancelled.push(tag);
                }
                Some(why) => report.failed.push((tag, why)),
            }
        }
        Ok(report)
    }

    fn lock(&self) -> MutexGuard<'_, LiveBook> {
        // A poisoned book is still the best record there is.
        self.book.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What [`LiveRoute::cancel_open`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CancelReport {
    pub cancelled: Vec<String>,
    /// Tags the broker did not cancel, and why.
    pub failed: Vec<(String, String)>,
}

/// An approved order for an armed strategy, ready for the broker. Only the
/// gateway mints one; [`Self::send`] spends it.
#[must_use = "a live dispatch does nothing until it is sent"]
pub struct LiveDispatch {
    route: Arc<LiveRoute>,
    strategy_id: i64,
    id: InstrumentId,
    order: ApprovedOrder,
    tag: OrderTag,
}

impl std::fmt::Debug for LiveDispatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveDispatch")
            .field("strategy_id", &self.strategy_id)
            .field("tag", &self.tag.as_str())
            .field("order", &self.order)
            .finish_non_exhaustive()
    }
}

impl LiveDispatch {
    pub(crate) fn new(
        route: Arc<LiveRoute>,
        strategy_id: i64,
        id: InstrumentId,
        order: ApprovedOrder,
        tag: OrderTag,
    ) -> Self {
        Self {
            route,
            strategy_id,
            id,
            order,
            tag,
        }
    }

    pub fn strategy_id(&self) -> i64 {
        self.strategy_id
    }

    pub fn tag(&self) -> &OrderTag {
        &self.tag
    }

    pub fn decision_id(&self) -> u64 {
        self.order.decision_id()
    }

    /// Sends the order to the broker, once.
    pub async fn send(self, now_ms: i64) -> Result<OrderAck, GatewayError> {
        let Self {
            route,
            strategy_id,
            id,
            order,
            tag,
        } = self;
        let decision_id = order.decision_id();
        let failed = |reason: String, ambiguous: bool| AuditEvent::OrderFailed {
            strategy_id,
            decision_id,
            tag: tag.as_str().to_owned(),
            reason,
            ambiguous,
        };
        if !order.is_valid_at(now_ms) {
            let _ = route.audit.record(
                now_ms,
                &failed("the approval expired before it was sent".into(), false),
            );
            return Err(GatewayError::ApprovalExpired);
        }
        let mut tracked = TrackedOrder {
            tag: tag.clone(),
            strategy_id,
            id,
            side: order.side(),
            quantity: order.quantity(),
            broker_id: None,
            status: None,
            filled_quantity: 0,
        };
        match route.broker.place(order, &tag).await {
            Ok(ack) => {
                tracked.broker_id = Some(ack.id.clone());
                tracked.status = Some(OrderStatus::Pending);
                route.lock().orders.push(tracked);
                route
                    .audit
                    .record(
                        now_ms,
                        &AuditEvent::OrderAcked {
                            strategy_id,
                            decision_id,
                            tag: tag.as_str().to_owned(),
                            broker_id: ack.id.to_string(),
                        },
                    )
                    .map_err(GatewayError::AuditFailed)?;
                Ok(ack)
            }
            Err(err) if err.placement_ambiguous() => {
                // Its fate is unknown: tracked without a broker id, so
                // reconciliation looks it up by tag. Never placed again.
                route.lock().orders.push(tracked);
                let _ = route.audit.record(now_ms, &failed(err.to_string(), true));
                Err(GatewayError::AmbiguousPlacement(tag.as_str().to_owned()))
            }
            Err(err) => {
                let _ = route.audit.record(now_ms, &failed(err.to_string(), false));
                Err(GatewayError::from(err))
            }
        }
    }
}
