use std::sync::Arc;
use std::time::Duration;
use std::net::SocketAddr;

use tokio::sync::{mpsc, Mutex};
use tokio::time;
use tracing::{debug, error, info, warn};

use rvoip_sip_core::{Request, Response, Message, Method, StatusCode};
use rvoip_transaction_core::{TransactionManager, TransactionEvent};

use crate::error::{Error, Result};
use crate::call::{CallEvent, CallDirection};
use super::events::SipClientEvent;

/// Trait to transform channels
pub(crate) trait ChannelTransformer<T, U> {
    fn with_transformer<F>(self, f: F) -> mpsc::Sender<T>
    where
        F: Fn(T) -> U + Send + 'static,
        T: Send + 'static,
        U: Send + 'static;
}

impl<T, U> ChannelTransformer<T, U> for mpsc::Sender<U> 
where
    T: Send + 'static,
    U: Send + 'static
{
    fn with_transformer<F>(self, f: F) -> mpsc::Sender<T>
    where
        F: Fn(T) -> U + Send + 'static,
    {
        let (tx, mut rx) = mpsc::channel(32);
        
        let tx_clone = self.clone();
        
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let transformed = f(msg);
                if let Err(e) = tx_clone.send(transformed).await {
                    error!("Error sending transformed message: {}", e);
                    break;
                }
            }
        });
        
        tx
    }
}

/// Add response headers to a response based on a request
pub(crate) fn add_response_headers(request: &Request, response: &mut Response) {
    // Copy relevant headers from request to response
    for header in &request.headers {
        match header.name {
            // Copy From header
            rvoip_sip_core::HeaderName::From => {
                response.headers.push(header.clone());
            },
            // Copy Call-ID header
            rvoip_sip_core::HeaderName::CallId => {
                response.headers.push(header.clone());
            },
            // Copy CSeq header
            rvoip_sip_core::HeaderName::CSeq => {
                response.headers.push(header.clone());
            },
            // Copy Via headers (in reverse order for responses)
            rvoip_sip_core::HeaderName::Via => {
                // Add Via headers in the same order as in the request
                response.headers.push(header.clone());
            },
            // Other headers are not copied
            _ => {}
        }
    }
} 