#[async_trait]
impl ClientTransaction for ClientInviteTransaction {
    // ... initiate ...

    /// Process an incoming response.
    async fn process_response(&mut self, response: Response) -> Result<()> {
        let status = response.status();
        let is_provisional = status.is_provisional();
        let is_success = status.is_success();
        let is_failure = !is_provisional && !is_success;
        let id = self.data.id.clone();

        match self.data.state {
            TransactionState::Calling => {
                self.cancel_timers();
                if is_provisional { /* ... (no change needed here) ... */ }
                else if is_success { /* ... (no change needed here) ... */ }
                else if is_failure { /* ... (no change needed here) ... */ }
            }
            TransactionState::Proceeding => {
                 if is_provisional { /* ... */ }
                 else if is_success { /* ... */ }
                 else if is_failure { /* ... */ }
            }
            TransactionState::Completed => {
                 if is_failure { /* ... */ }
                 else { /* ... */ }
            }
            // Use status in log message
            TransactionState::Terminated | TransactionState::Initial | TransactionState::Trying | TransactionState::Confirmed => {
                 warn!(id=%id, state=?self.data.state, status=%status, "Received response in unexpected state");
            }
        }
        Ok(())
    }
}

#[async_trait]
impl ClientTransaction for ClientNonInviteTransaction {
     // ... initiate ...

     /// Process an incoming response.
    async fn process_response(&mut self, response: Response) -> Result<()> {
        let status = response.status();
        let is_provisional = status.is_provisional();
        let is_success = status.is_success();
        let is_failure = !is_provisional && !is_success;
        let is_final = !is_provisional;

        let id = self.data.id.clone();

        match self.data.state {
             TransactionState::Trying => {
                 self.cancel_timers();
                 if is_provisional { /* ... */ }
                 else if is_final { /* ... */
                     if is_success { /* ... */ }
                     else { /* is_failure */ /* ... */ }
                 }
             }
             TransactionState::Proceeding => {
                 if is_provisional { /* ... */ }
                 else if is_final { /* ... */
                     if is_success { /* ... */ }
                     else { /* is_failure */ /* ... */ }
                 }
             }
             // Use status in log message
             TransactionState::Completed | TransactionState::Terminated | TransactionState::Initial | TransactionState::Calling | TransactionState::Confirmed => {
                 trace!(id=%id, state=?self.data.state, %status, "Ignoring response");
             }
         }
         Ok(())
     }
}

// In server.rs
#[async_trait]
impl ServerTransaction for ServerInviteTransaction {
    // ... process_request (already fixed) ...

    /// Send a response (1xx, 2xx, 3xx-6xx).
    async fn send_response(&mut self, response: Response) -> Result<()> {
         let status = response.status();
         let is_provisional = status.is_provisional();
         let is_success = status.is_success();
         let is_final = !is_provisional;

         let id = self.data.id.clone();

         match self.data.state {
              TransactionState::Proceeding => {
                  self.data.last_response = Some(response.clone());
                  self.data.transport.send_message(
                      Message::Response(response.clone()),
                      self.data.remote_addr
                  ).await.map_err(|e| Error::TransportError(e.to_string()))?;

                  if is_final {
                      if is_success { // 2xx
                          debug!(id=%id, %status, "Sent 2xx final response, terminating transaction");
                          self.transition_to(TransactionState::Terminated).await?;
                          self.data.events_tx.send(TransactionEvent::FinalResponseSent {
                             transaction_id: id,
                             response,
                         }).await?;
                      } else { // 3xx-6xx
                           debug!(id=%id, %status, "Sent non-2xx final response, moving to Completed");
                           self.transition_to(TransactionState::Completed).await?;
                            self.data.events_tx.send(TransactionEvent::FinalResponseSent {
                               transaction_id: id,
                               response,
                           }).await?;
                      }
                } else { // is_provisional
                        debug!(id=%id, %status, "Sent another provisional response");
                        self.data.events_tx.send(TransactionEvent::ProvisionalResponseSent {
                            transaction_id: id,
                            response,
                         }).await?;
                  }
              }
              TransactionState::Initial => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Initial state");
                   return Err(Error::InvalidStateTransition("Cannot send response in Initial state".to_string()));
              }
              TransactionState::Trying => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Trying state (server INVITE)");
                   return Err(Error::InvalidStateTransition("Cannot send response in Trying state".to_string()));
              }
              TransactionState::Calling => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Calling state (server INVITE)");
                   return Err(Error::InvalidStateTransition("Cannot send response in Calling state".to_string()));
              }
              TransactionState::Completed => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Completed state (already sent final)");
                   return Err(Error::InvalidStateTransition("Cannot send response in Completed state".to_string()));
              }
              TransactionState::Confirmed => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Confirmed state (already ACKed)");
                   return Err(Error::InvalidStateTransition("Cannot send response in Confirmed state".to_string()));
              }
              TransactionState::Terminated => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Terminated state");
                   return Err(Error::InvalidStateTransition("Cannot send response in Terminated state".to_string()));
               }
         }
         Ok(())
     }
}

#[async_trait]
impl ServerTransaction for ServerNonInviteTransaction {
    /// Process a retransmitted request.
    async fn process_request(&mut self, request: Request) -> Result<()> {
         let id = self.data.id.clone();
         // ... (method check) ...

         match self.data.state {
              TransactionState::Trying | TransactionState::Proceeding => {
                   if let Some(resp) = &self.data.last_response {
                       if resp.status().is_provisional() { /* ... */ }
                   }
              }
            TransactionState::Completed => {
                   if let Some(resp) = &self.data.last_response {
                       // is_final == !is_provisional
                       if !resp.status().is_provisional() { /* ... */ }
                   }
              }
             // Use status in logs for added arms
             TransactionState::Initial => {
                  warn!(id=%id, state=?self.data.state, method=%request.method(), "Ignoring request retransmission in Initial state");
             }
              TransactionState::Calling => {
                 warn!(id=%id, state=?self.data.state, method=%request.method(), "Ignoring request retransmission in Calling state (non-INVITE)");
              }
              TransactionState::Confirmed => {
                 warn!(id=%id, state=?self.data.state, method=%request.method(), "Ignoring request retransmission in Confirmed state (non-INVITE)");
              }
             TransactionState::Terminated => {
                  trace!(id=%id, state=?self.data.state, "Ignoring request retransmission in Terminated state");
              }
         }
         Ok(())
     }

    /// Send a response (1xx, 2xx-6xx).
    async fn send_response(&mut self, response: Response) -> Result<()> {
         let status = response.status();
         let is_provisional = status.is_provisional();
         let is_final = !is_provisional;

         let id = self.data.id.clone();

         match self.data.state {
              TransactionState::Trying => {
                  // ... (send, transition, event) ...
              }
            TransactionState::Proceeding => {
                  // ... (send, transition, event) ...
              }
             // Use status in logs for added arms
             TransactionState::Initial => {
                  error!(id=%id, state=?self.data.state, %status, "Cannot send response in Initial state");
                  return Err(Error::InvalidStateTransition("Cannot send response in Initial state".to_string()));
             }
              TransactionState::Calling => {
                  error!(id=%id, state=?self.data.state, %status, "Cannot send response in Calling state (non-INVITE)");
                  return Err(Error::InvalidStateTransition("Cannot send response in Calling state".to_string()));
              }
              TransactionState::Confirmed => {
                  error!(id=%id, state=?self.data.state, %status, "Cannot send response in Confirmed state (non-INVITE)");
                  return Err(Error::InvalidStateTransition("Cannot send response in Confirmed state".to_string()));
              }
              TransactionState::Completed => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Completed state");
                   return Err(Error::InvalidStateTransition("Cannot send response in Completed state".to_string()));
              }
              TransactionState::Terminated => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Terminated state");
                   return Err(Error::InvalidStateTransition("Cannot send response in Terminated state".to_string()));
              }
         }
         Ok(())
     }
} 