//! Subscription management for presence

use dashmap::DashMap;
use std::sync::Arc;
use chrono::Utc;
use uuid::Uuid;
use crate::types::{Subscription, SubscriptionState};
use crate::error::{RegistrarError, Result};

/// Manages presence subscriptions
pub struct SubscriptionManager {
    /// Map of subscription_id to subscription
    subscriptions: Arc<DashMap<String, Subscription>>,
    
    /// Map of target user to list of subscription IDs watching them
    watchers: Arc<DashMap<String, Vec<String>>>,
    
    /// Map of subscriber to list of subscription IDs they have
    watching: Arc<DashMap<String, Vec<String>>>,
}

impl SubscriptionManager {
    pub fn new() -> Self {
        Self {
            subscriptions: Arc::new(DashMap::new()),
            watchers: Arc::new(DashMap::new()),
            watching: Arc::new(DashMap::new()),
        }
    }
    
    /// Add a new subscription
    pub async fn add_subscription(
        &self,
        subscriber: &str,
        target: &str,
        expires: u32,
    ) -> Result<String> {
        let subscription_id = Uuid::new_v4().to_string();
        let expires_at = Utc::now() + chrono::Duration::seconds(expires as i64);
        
        let subscription = Subscription {
            id: subscription_id.clone(),
            subscriber: subscriber.to_string(),
            target: target.to_string(),
            state: SubscriptionState::Active,
            expires_at,
            event_id: 0,
            accept_types: vec!["application/pidf+xml".to_string()],
            created_at: Utc::now(),
            last_notify: None,
            notify_count: 0,
        };
        
        // Store subscription
        self.subscriptions.insert(subscription_id.clone(), subscription);
        
        // Update watchers index
        self.watchers
            .entry(target.to_string())
            .and_modify(|subs| subs.push(subscription_id.clone()))
            .or_insert(vec![subscription_id.clone()]);
        
        // Update watching index
        self.watching
            .entry(subscriber.to_string())
            .and_modify(|subs| subs.push(subscription_id.clone()))
            .or_insert(vec![subscription_id.clone()]);
        
        Ok(subscription_id)
    }
    
    /// Remove a subscription
    pub async fn remove_subscription(&self, subscription_id: &str) -> Result<()> {
        if let Some((_, subscription)) = self.subscriptions.remove(subscription_id) {
            // Remove from watchers index
            if let Some(mut watchers) = self.watchers.get_mut(&subscription.target) {
                watchers.retain(|id| id != subscription_id);
                if watchers.is_empty() {
                    drop(watchers);
                    self.watchers.remove(&subscription.target);
                }
            }
            
            // Remove from watching index
            if let Some(mut watching) = self.watching.get_mut(&subscription.subscriber) {
                watching.retain(|id| id != subscription_id);
                if watching.is_empty() {
                    drop(watching);
                    self.watching.remove(&subscription.subscriber);
                }
            }
            
            Ok(())
        } else {
            Err(RegistrarError::SubscriptionNotFound(subscription_id.to_string()))
        }
    }
    
    /// Get all subscribers watching a user
    pub async fn get_subscribers(&self, target: &str) -> Result<Vec<String>> {
        Ok(self.watchers
            .get(target)
            .map(|entry| {
                entry.iter()
                    .filter_map(|sub_id| {
                        self.subscriptions.get(sub_id)
                            .map(|sub| sub.subscriber.clone())
                    })
                    .collect()
            })
            .unwrap_or_default())
    }
    
    /// Get all users that a subscriber is watching
    pub async fn get_subscriptions(&self, subscriber: &str) -> Result<Vec<String>> {
        Ok(self.watching
            .get(subscriber)
            .map(|entry| {
                entry.iter()
                    .filter_map(|sub_id| {
                        self.subscriptions.get(sub_id)
                            .map(|sub| sub.target.clone())
                    })
                    .collect()
            })
            .unwrap_or_default())
    }
    
    /// Get subscription details
    pub async fn get_subscription(&self, subscription_id: &str) -> Result<Subscription> {
        self.subscriptions
            .get(subscription_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| RegistrarError::SubscriptionNotFound(subscription_id.to_string()))
    }
    
    /// Notify subscribers of a presence change
    pub async fn notify_subscribers(&self, target: &str) -> Result<Vec<String>> {
        let notified = Vec::new();
        
        if let Some(watchers) = self.watchers.get(target) {
            for sub_id in watchers.iter() {
                if let Some(mut subscription) = self.subscriptions.get_mut(sub_id) {
                    subscription.notify_count += 1;
                    // Actual notification would be sent through session-core
                }
            }
        }
        
        Ok(notified)
    }
    
    /// Expire old subscriptions
    pub async fn expire_subscriptions(&self) -> Vec<String> {
        let mut expired = Vec::new();
        let now = Utc::now();
        
        // Find expired subscriptions
        let to_expire: Vec<String> = self.subscriptions
            .iter()
            .filter(|entry| entry.expires_at < now)
            .map(|entry| entry.id.clone())
            .collect();
        
        // Remove expired subscriptions
        for sub_id in to_expire {
            if let Ok(_) = self.remove_subscription(&sub_id).await {
                expired.push(sub_id);
            }
        }
        
        expired
    }
}