//! Call routing logic for the call center
//!
//! This module implements the sophisticated routing engine that determines
//! how incoming calls are distributed to agents based on skills, availability,
//! and business rules.

use std::sync::Arc;
use tracing::{debug, info};
use rvoip_session_core::{IncomingCall, SessionId};

use crate::agent::{AgentId, AgentStatus};
use crate::error::Result as CallCenterResult;
use crate::queue::QueuedCall;
use super::core::CallCenterEngine;
use super::types::{CustomerType, RoutingDecision, AgentInfo};

impl CallCenterEngine {
    /// Analyze customer information to determine routing requirements
    pub(super) async fn analyze_customer_info(&self, call: &IncomingCall) -> (CustomerType, u8, Vec<String>) {
        // This would integrate with CRM systems, customer databases, etc.
        // For now, use simple heuristics based on caller information
        
        let caller_number = &call.from;
        
        // Determine customer type (would be from database lookup in production)
        let customer_type = if caller_number.contains("+1800") || caller_number.contains("vip") {
            CustomerType::VIP
        } else if caller_number.contains("+1900") {
            CustomerType::Premium  
        } else if caller_number.contains("trial") {
            CustomerType::Trial
        } else {
            CustomerType::Standard
        };
        
        // Determine priority (0 = highest, 255 = lowest)
        let priority = match customer_type {
            CustomerType::VIP => 0,
            CustomerType::Premium => 10,
            CustomerType::Standard => 50,
            CustomerType::Trial => 100,
        };
        
        // Determine required skills (would be more sophisticated in production)
        let required_skills = if caller_number.contains("support") {
            vec!["technical_support".to_string()]
        } else if caller_number.contains("sales") {
            vec!["sales".to_string()]
        } else if caller_number.contains("billing") {
            vec!["billing".to_string()]
        } else {
            vec!["general".to_string()]
        };
        
        debug!("📊 Customer analysis - Type: {:?}, Priority: {}, Skills: {:?}", 
               customer_type, priority, required_skills);
        
        (customer_type, priority, required_skills)
    }
    
    /// Make intelligent routing decision based on multiple factors
    pub(super) async fn make_routing_decision(
        &self,
        session_id: &SessionId,
        customer_type: &CustomerType,
        priority: u8,
        required_skills: &[String],
    ) -> CallCenterResult<RoutingDecision> {
        
        // **STEP 1**: Try to find available agents with matching skills
        if let Some(agent_id) = self.find_best_available_agent(required_skills, priority).await {
            return Ok(RoutingDecision::DirectToAgent {
                agent_id,
                reason: "Skilled agent available".to_string(),
            });
        }
        
        // **STEP 2**: Check if we should queue based on customer type and current load
        let queue_decision = self.determine_queue_strategy(customer_type, priority, required_skills).await;
        
        // **STEP 3**: Check for overflow conditions
        if self.should_overflow_call(customer_type, priority).await {
            return Ok(RoutingDecision::Overflow {
                target_queue: "overflow".to_string(),
                reason: "Primary queues full".to_string(),
            });
        }
        
        // **STEP 4**: Default to queueing with appropriate queue selection
        Ok(queue_decision)
    }
    
    /// Find the best available agent based on skills and performance
    pub(super) async fn find_best_available_agent(&self, required_skills: &[String], priority: u8) -> Option<AgentId> {
        // Find agents with matching skills and availability
        let mut suitable_agents: Vec<(AgentId, AgentInfo)> = self.available_agents
            .iter()
            .filter(|entry| {
                let agent_info = entry.value();
                // Check if agent is available
                matches!(agent_info.status, AgentStatus::Available) &&
                // Check if agent has capacity
                agent_info.current_calls < agent_info.max_calls &&
                // Check skill match (if no specific skills required, any agent works)
                (required_skills.is_empty() || 
                 required_skills.iter().any(|skill| agent_info.skills.contains(skill)))
            })
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        
        if suitable_agents.is_empty() {
            debug!("❌ No suitable agents found for skills: {:?}", required_skills);
            return None;
        }
        
        // Sort by performance score and last call end time (round-robin effect)
        suitable_agents.sort_by(|(_, a), (_, b)| {
            // Primary: performance score (higher is better)
            let score_cmp = b.performance_score.partial_cmp(&a.performance_score).unwrap_or(std::cmp::Ordering::Equal);
            if score_cmp != std::cmp::Ordering::Equal {
                return score_cmp;
            }
            
            // Secondary: longest idle time (for round-robin)
            match (&a.last_call_end, &b.last_call_end) {
                (Some(a_end), Some(b_end)) => a_end.cmp(b_end), // Earlier end time first
                (None, Some(_)) => std::cmp::Ordering::Less,     // Never handled call first
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });
        
        let best_agent = suitable_agents.first().map(|(agent_id, _)| agent_id.clone());
        
        if let Some(ref agent_id) = best_agent {
            info!("🎯 Selected agent {} for skills {:?} (priority {})", agent_id, required_skills, priority);
        }
        
        best_agent
    }
    
    /// Determine appropriate queue strategy
    pub(super) async fn determine_queue_strategy(
        &self,
        customer_type: &CustomerType,
        priority: u8,
        required_skills: &[String],
    ) -> RoutingDecision {
        
        // Select queue based on skills and customer type
        let queue_id = if required_skills.contains(&"technical_support".to_string()) {
            "support"
        } else if required_skills.contains(&"sales".to_string()) {
            "sales"
        } else if required_skills.contains(&"billing".to_string()) {
            "billing"
        } else {
            match customer_type {
                CustomerType::VIP => "vip",
                CustomerType::Premium => "premium",
                _ => "general",
            }
        };
        
        RoutingDecision::Queue {
            queue_id: queue_id.to_string(),
            priority,
            reason: format!("Queue selected for {} customer with skills {:?}", 
                          format!("{:?}", customer_type).to_lowercase(), required_skills),
        }
    }
    
    /// Check if call should be overflowed to alternate routing
    pub(super) async fn should_overflow_call(&self, customer_type: &CustomerType, priority: u8) -> bool {
        // **FUTURE**: Implement sophisticated overflow logic
        // For now, simple check based on queue lengths
        
        let queue_manager = self.queue_manager.read().await;
        
        // Check total queue load (simplified)
        // In production, this would check specific queue capacities, wait times, etc.
        
        false // For now, don't overflow
    }
    
    /// Ensure a queue exists, create if necessary
    pub(super) async fn ensure_queue_exists(&self, queue_id: &str) -> CallCenterResult<()> {
        let mut queue_manager = self.queue_manager.write().await;
        
        // Check if queue exists (this is a simplified check)
        // In production, we'd have better queue existence checking
        
        // Create standard queues if they don't exist
        let standard_queues = vec![
            ("general", "General Support", 100),
            ("sales", "Sales", 50),
            ("support", "Technical Support", 75),
            ("billing", "Billing", 30),
            ("vip", "VIP Support", 20),
            ("premium", "Premium Support", 40),
            ("overflow", "Overflow Queue", 200),
        ];
        
        for (id, name, max_size) in standard_queues {
            if id == queue_id {
                // Try to create queue (will succeed if doesn't exist)
                let _ = queue_manager.create_queue(id.to_string(), name.to_string(), max_size);
                break;
            }
        }
        
        Ok(())
    }
    
    /// Monitor queue for agent availability
    pub(super) async fn monitor_queue_for_agents(&self, queue_id: String) {
        // Spawn background task to monitor queue and assign agents when available
        let engine = Arc::new(self.clone());
        tokio::spawn(async move {
            // **FUTURE**: Implement intelligent queue monitoring
            // For now, just log that we're monitoring
            debug!("👁️ Monitoring queue {} for agent availability", queue_id);
            
            // This would periodically check for available agents and dequeue calls
            // Implementation would go here...
        });
    }
} 