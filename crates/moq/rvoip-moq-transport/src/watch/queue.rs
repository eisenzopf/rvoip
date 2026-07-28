// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::State;
use futures::channel::oneshot;
use std::collections::VecDeque;

pub struct Queue<T> {
    state: State<VecDeque<(T, Option<oneshot::Sender<()>>)>>, // store optional notifier per item
    capacity: Option<usize>,
}

impl<T> Queue<T> {
    /// Create a queue which retains at most `capacity` items.
    pub fn bounded(capacity: usize) -> Self {
        assert!(capacity > 0, "queue capacity must be greater than zero");
        Self {
            state: State::new(Default::default()),
            capacity: Some(capacity),
        }
    }

    /// Push an item onto the queue. Returns `Err(item)` when the queue is
    /// closed or already at its configured capacity.
    pub fn push(&mut self, item: T) -> Result<(), T> {
        match self.state.lock_mut() {
            Some(mut state) if self.capacity.is_none_or(|capacity| state.len() < capacity) => {
                state.push_back((item, None));
            }
            Some(_) => return Err(item),
            None => return Err(item),
        };

        Ok(())
    }

    /// Pop an item from the queue, waiting if necessary.
    pub async fn pop(&mut self) -> Option<T> {
        loop {
            // Scope 1: try to pop an item
            {
                let queue = self.state.lock();
                if !queue.is_empty() {
                    // Accepted items remain readable after the producer side
                    // closes. Once the queue is drained, `modified()` below
                    // observes closure and returns `None`.
                    if let Some((item, notifier)) = {
                        let mut state_mut = queue.into_mut_after_close();
                        state_mut.pop_front()
                    } {
                        if let Some(tx) = notifier {
                            let _ = tx.send(()); // notify waiter
                        }
                        return Some(item);
                    }
                }
            }

            // Scope 2: wait for modifications
            let queue = self.state.lock();
            queue.modified()?.await;
        }
    }

    /// Drop the state
    pub fn close(self) -> Vec<T> {
        // Drain the queue of any remaining entries
        let res = match self.state.lock_mut() {
            Some(mut queue) => queue.drain(..).map(|(item, _)| item).collect(),
            _ => Vec::new(),
        };

        // Prevent any new entries from being added
        drop(self.state);

        res
    }

    /// Push an item and wait until it is popped.
    /// Returns Ok(()) if the item was successfully popped.
    /// Returns Err(()) if the queue was closed before the item could be confirmed popped.
    pub async fn push_and_wait_until_popped(&mut self, item: T) -> Result<(), ()> {
        // Create a oneshot channel
        let (tx, rx) = oneshot::channel();

        // Push the item along with the sender
        match self.state.lock_mut() {
            Some(mut state) if self.capacity.is_none_or(|capacity| state.len() < capacity) => {
                state.push_back((item, Some(tx)));
            }
            Some(_) => return Err(()),
            None => return Err(()), // Queue already closed before push
        }

        // Wait until the item is popped.
        // If we receive Canceled, it means the sender was dropped without sending,
        // which indicates the queue was closed while we were waiting.
        rx.await.map_err(|_| ())
    }

    /// Split the queue into two handles that share the same underlying state.
    pub fn split(self) -> (Self, Self) {
        let state = self.state.split();
        (
            Self {
                state: state.0,
                capacity: self.capacity,
            },
            Self {
                state: state.1,
                capacity: self.capacity,
            },
        )
    }

    /// Number of retained items, including items whose producer is waiting
    /// for confirmation that they were popped.
    pub fn len(&self) -> usize {
        self.state.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> Option<usize> {
        self.capacity
    }

    /// Remove all queued items matching `predicate` and return them in queue
    /// order. Any producer waiting for a removed item observes cancellation.
    pub fn remove_where(&mut self, mut predicate: impl FnMut(&T) -> bool) -> Vec<T> {
        let Some(mut queue) = self.state.lock_mut() else {
            return Vec::new();
        };
        let mut removed = Vec::new();
        let mut index = 0;
        while index < queue.len() {
            if predicate(&queue[index].0) {
                if let Some(entry) = queue.remove(index) {
                    removed.push(entry);
                }
            } else {
                index += 1;
            }
        }
        drop(queue);
        removed.into_iter().map(|(item, _)| item).collect()
    }
}

impl<T> Clone for Queue<T> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            capacity: self.capacity,
        }
    }
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            state: State::new(Default::default()),
            capacity: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_queue_rejects_n_plus_one_and_reuses_capacity() {
        let (mut producer, mut consumer) = Queue::bounded(2).split();
        assert_eq!(producer.capacity(), Some(2));
        assert!(producer.push(1).is_ok());
        assert!(producer.push(2).is_ok());
        assert_eq!(producer.push(3), Err(3));
        assert_eq!(consumer.len(), 2);
        assert_eq!(consumer.pop().await, Some(1));
        assert!(producer.push(3).is_ok());
        assert_eq!(consumer.pop().await, Some(2));
        assert_eq!(consumer.pop().await, Some(3));
        assert!(consumer.is_empty());
    }

    #[tokio::test]
    async fn consumer_drains_accepted_items_before_observing_producer_close() {
        let (mut producer, mut consumer) = Queue::bounded(2).split();
        producer.push(1).unwrap();
        producer.push(2).unwrap();
        drop(producer);

        assert_eq!(consumer.pop().await, Some(1));
        assert_eq!(consumer.pop().await, Some(2));
        assert_eq!(consumer.pop().await, None);
    }

    #[test]
    fn remove_where_preserves_order_and_releases_capacity() {
        let mut queue = Queue::bounded(4);
        for value in 0..4 {
            queue.push(value).unwrap();
        }
        assert_eq!(queue.remove_where(|value| value % 2 == 0), vec![0, 2]);
        assert_eq!(queue.len(), 2);
        assert!(queue.push(4).is_ok());
        assert!(queue.push(5).is_ok());
        assert_eq!(queue.push(6), Err(6));
        assert_eq!(queue.close(), vec![1, 3, 4, 5]);
    }

    #[tokio::test]
    async fn removing_waited_item_notifies_producer_with_cancellation() {
        let (mut producer, mut consumer) = Queue::bounded(1).split();
        let waiter = tokio::spawn(async move { producer.push_and_wait_until_popped(7).await });
        tokio::task::yield_now().await;
        assert_eq!(consumer.remove_where(|value| *value == 7), vec![7]);
        assert_eq!(waiter.await.unwrap(), Err(()));
        assert!(consumer.is_empty());
    }

    #[test]
    fn flood_never_grows_beyond_capacity() {
        let mut queue = Queue::bounded(8);
        let mut rejected = 0;
        for value in 0..10_000 {
            if queue.push(value).is_err() {
                rejected += 1;
            }
        }
        assert_eq!(queue.len(), 8);
        assert_eq!(rejected, 9_992);
    }
}
