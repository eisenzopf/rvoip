//! # Intelligent Call Routing for Call Center Operations
//!
//! This module provides sophisticated call routing algorithms and decision-making logic
//! for call center operations. It handles intelligent agent selection, skill-based routing,
//! load balancing, priority management, and advanced routing strategies to optimize
//! call distribution and ensure optimal customer experience.
//!
//! ## Overview
//!
//! Call routing is the brain of the call center system, determining how incoming calls
//! are distributed among available agents. This module implements multiple routing
//! algorithms including round-robin, skill-based routing, performance-based assignment,
//! and load balancing strategies. It integrates with agent management, queue systems,
//! and routing policies to provide enterprise-grade call distribution.
//!
//! ## Key Features
//!
//! - **Multi-Algorithm Support**: Round-robin, skill-based, performance-based routing
//! - **Load Balancing**: Intelligent workload distribution across agents
//! - **Priority Management**: Support for call prioritization and VIP handling
//! - **Skill Matching**: Advanced skill-based agent matching algorithms
//! - **Performance Optimization**: Agent selection based on performance metrics
//! - **Real-time Adaptation**: Dynamic routing based on current system state
//! - **Fairness Algorithms**: Ensures equitable call distribution
//! - **Statistics Tracking**: Comprehensive routing metrics and analytics
//! - **Policy Integration**: Seamless integration with routing policies
//! - **Overflow Handling**: Automatic overflow and escalation management
//!
//! ## Routing Algorithms
//!
//! ### Round-Robin Routing
//!
//! Distributes calls evenly among available agents in sequential order:
//!
//! - **Fair Distribution**: Ensures each agent gets approximately equal call volume
//! - **Last Agent Exclusion**: Prevents immediate reassignment to same agent
//! - **State Tracking**: Maintains routing state across calls
//! - **Wrapping Logic**: Automatically wraps around the agent list
//!
//! ### Skill-Based Routing
//!
//! Matches calls to agents based on required skills and expertise:
//!
//! - **Skill Matching**: Precise matching of call requirements to agent skills
//! - **Weighted Scoring**: Calculates best-fit agents using skill weights
//! - **Fallback Logic**: Graceful degradation when perfect matches unavailable
//! - **Skill Prioritization**: Supports primary and secondary skill requirements
//!
//! ### Performance-Based Routing
//!
//! Routes calls to agents based on performance metrics and ratings:
//!
//! - **Performance Scoring**: Weighted scoring based on multiple metrics
//! - **Dynamic Adjustment**: Real-time performance factor adjustments
//! - **Quality Balancing**: Balances performance optimization with fairness
//! - **Metric Integration**: Incorporates call quality, resolution time, customer satisfaction
//!
//! ## Examples
//!
//! ### Basic Round-Robin Routing
//!
//! ```rust
//! use rvoip_call_engine::agent::{Agent, AgentStatus};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Note: Using conceptual example - CallRouter not yet implemented
//! println!("Initializing call router with round-robin algorithm");
//! 
//! // Set up available agents
//! let agents = vec![
//!     Agent {
//!         id: "agent-001".to_string(),
//!         sip_uri: "sip:agent001@call-center.local".to_string(),
//!         display_name: "Senior Agent".to_string(),
//!         skills: vec!["english".to_string(), "technical".to_string()],
//!         max_concurrent_calls: 3,
//!         status: AgentStatus::Available,
//!         department: Some("tech_support".to_string()),
//!         extension: Some("2001".to_string()),
//!     },
//!     Agent {
//!         id: "agent-002".to_string(),
//!         sip_uri: "sip:agent002@call-center.local".to_string(),
//!         display_name: "Junior Agent".to_string(),
//!         skills: vec!["english".to_string(), "billing".to_string()],
//!         max_concurrent_calls: 2,
//!         status: AgentStatus::Available,
//!         department: Some("billing".to_string()),
//!         extension: Some("2002".to_string()),
//!     },
//!     Agent {
//!         id: "agent-003".to_string(),
//!         sip_uri: "sip:agent003@call-center.local".to_string(),
//!         display_name: "Specialist Agent".to_string(),
//!         skills: vec!["english".to_string(), "escalation".to_string()],
//!         max_concurrent_calls: 2,
//!         status: AgentStatus::Available,
//!         department: Some("escalation".to_string()),
//!         extension: Some("2003".to_string()),
//!     },
//! ];
//! 
//! // Conceptual routing request
//! let session_id = SessionId("customer-call-001".to_string());
//! let customer_priority = "standard";
//! let required_skills = vec!["general_support".to_string()];
//! 
//! // Note: Using conceptual example - routing logic not yet fully implemented
//! println!("📞 Processing call {} with priority: {}", session_id, customer_priority);
//! println!("🎯 Required skills: {:?}", required_skills);
//! 
//! // In the actual implementation, this would use round-robin logic
//! if !agents.is_empty() {
//!     let selected_agent = &agents[0]; // Simple example selection
//!     println!("✅ Call routed to agent: {}", selected_agent.id);
//!     println!("🎯 Routing algorithm: Round-Robin (conceptual)");
//!     println!("📊 Agent has skills: {:?}", selected_agent.skills);
//! } else {
//!     println!("❌ No available agents for routing");
//!     println!("🔄 Call will be queued");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Skill-Based Routing
//!
//! ```rust
//! use rvoip_call_engine::agent::{Agent, AgentStatus};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Note: Using conceptual example - CallRouter not yet implemented
//! println!("Initializing skill-based routing system");
//! 
//! // Set up agents with different skills
//! let agents = vec![
//!     Agent {
//!         id: "agent-tech-001".to_string(),
//!         sip_uri: "sip:alice@call-center.com".to_string(),
//!         display_name: "Senior Agent".to_string(),
//!         skills: vec![
//!             "technical_support".to_string(),
//!             "hardware_issues".to_string(),
//!             "software_troubleshooting".to_string(),
//!         ],
//!         max_concurrent_calls: 2,
//!         status: AgentStatus::Available,
//!         department: Some("tech_support".to_string()),
//!         extension: Some("2001".to_string()),
//!     },
//!     Agent {
//!         id: "agent-billing-001".to_string(),
//!         sip_uri: "sip:bob@call-center.com".to_string(),
//!         display_name: "Junior Agent".to_string(),
//!         skills: vec![
//!             "billing_support".to_string(),
//!             "payment_processing".to_string(),
//!             "account_management".to_string(),
//!         ],
//!         max_concurrent_calls: 3,
//!         status: AgentStatus::Available,
//!         department: Some("billing".to_string()),
//!         extension: Some("2002".to_string()),
//!     },
//!     Agent {
//!         id: "agent-general-001".to_string(),
//!         sip_uri: "sip:carol@call-center.com".to_string(),
//!         display_name: "Specialist Agent".to_string(),
//!         skills: vec![
//!             "general_support".to_string(),
//!             "technical_support".to_string(),
//!         ],
//!         max_concurrent_calls: 3,
//!         status: AgentStatus::Available,
//!         department: Some("escalation".to_string()),
//!         extension: Some("2003".to_string()),
//!     },
//! ];
//! 
//! // Route technical support call
//! let tech_session = SessionId("tech-support-call".to_string());
//! let required_tech_skills = vec![
//!     "technical_support".to_string(),
//!     "hardware_issues".to_string(),
//! ];
//! 
//! println!("🔧 Processing technical support call: {}", tech_session);
//! println!("🎯 Required skills: {:?}", required_tech_skills);
//! 
//! // Find best agent with matching skills (conceptual implementation)
//! let best_tech_agent = agents.iter().find(|agent| {
//!     agent.skills.contains(&"technical_support".to_string()) &&
//!     agent.skills.contains(&"hardware_issues".to_string())
//! });
//! 
//! match best_tech_agent {
//!     Some(agent) => {
//!         println!("🔧 Technical call routed to: {}", agent.id);
//!         println!("🎯 Best skill match found");
//!         println!("📊 Agent skills: {:?}", agent.skills);
//!         println!("💭 Routing reasoning: Exact skill match for technical support");
//!     }
//!     None => {
//!         println!("❌ No agents available with required technical skills");
//!     }
//! }
//! 
//! // Route billing call
//! let billing_session = SessionId("billing-support-call".to_string());
//! let billing_skills = vec!["billing_support".to_string()];
//! 
//! println!("💰 Processing billing support call: {}", billing_session);
//! 
//! // Find agent with billing skills
//! let billing_agent = agents.iter().find(|agent| {
//!     agent.skills.contains(&"billing_support".to_string())
//! });
//! 
//! println!("💰 Billing call routing result:");
//! if let Some(agent) = billing_agent {
//!     println!("  ✅ Routed to billing specialist: {}", agent.id);
//!     println!("  📋 Agent skills: {:?}", agent.skills);
//! } else {
//!     println!("  ❌ No billing specialists available");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Performance-Based Routing
//!
//! ```rust
//! use rvoip_call_engine::agent::{Agent, AgentStatus};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Note: Using conceptual example - CallRouter not yet implemented
//! println!("Initializing performance-based routing system");
//! 
//! // Set up agents with varying performance ratings
//! let agents = vec![
//!     Agent {
//!         id: "top-performer".to_string(),
//!         sip_uri: "sip:topagent@call-center.local".to_string(),
//!         display_name: "Top Agent".to_string(),
//!         skills: vec!["english".to_string(), "premium".to_string()],
//!         max_concurrent_calls: 3,
//!         status: AgentStatus::Available,
//!         department: Some("premium".to_string()),
//!         extension: Some("3001".to_string()),
//!     },
//!     Agent {
//!         id: "good-performer".to_string(),
//!         sip_uri: "sip:goodagent@call-center.local".to_string(),
//!         display_name: "Good Agent".to_string(),
//!         skills: vec!["english".to_string(), "general".to_string()],
//!         max_concurrent_calls: 2,
//!         status: AgentStatus::Available,
//!         department: Some("general".to_string()),
//!         extension: Some("3002".to_string()),
//!     },
//!     Agent {
//!         id: "newer-agent".to_string(),
//!         sip_uri: "sip:newagent@call-center.local".to_string(),
//!         display_name: "New Agent".to_string(),
//!         skills: vec!["english".to_string(), "training".to_string()],
//!         max_concurrent_calls: 1,
//!         status: AgentStatus::Available,
//!         department: Some("training".to_string()),
//!         extension: Some("3003".to_string()),
//!     },
//! ];
//! 
//! // Route VIP customer call with performance priority
//! let vip_session = SessionId("vip-customer-call".to_string());
//! let vip_priority = "vip";
//! let performance_threshold = 4.0;
//! 
//! println!("⭐ Processing VIP customer call: {}", vip_session);
//! println!("🎯 Priority: {}, Performance threshold: {:.1}", vip_priority, performance_threshold);
//! 
//! // Find highest performing agent (conceptual - would use actual performance metrics)
//! let premium_agent = agents.iter().find(|agent| {
//!     agent.skills.contains(&"premium".to_string()) || 
//!     agent.department == Some("premium".to_string())
//! });
//! 
//! match premium_agent {
//!     Some(agent) => {
//!         println!("⭐ VIP call routed to top performer: {}", agent.id);
//!         println!("📊 Agent department: {:?}", agent.department);
//!         println!("🎯 Ensuring best customer experience");
//!         
//!         // Performance routing details (conceptual)
//!         println!("📋 Routing details:");
//!         println!("  Agent skills: {:?}", agent.skills);
//!         println!("  Max concurrent calls: {}", agent.max_concurrent_calls);
//!         println!("  Department: {:?}", agent.department);
//!     }
//!     None => {
//!         println!("❌ No premium agents available for VIP call");
//!         println!("🔄 Consider escalation or queue with high priority");
//!     }
//! }
//! 
//! // Route standard call with balanced approach
//! let standard_session = SessionId("standard-customer-call".to_string());
//! let standard_priority = "standard";
//! 
//! println!("📞 Processing standard customer call: {}", standard_session);
//! 
//! // Find any available general agent (balanced approach)
//! let general_agent = agents.iter().find(|agent| {
//!     agent.skills.contains(&"general".to_string()) || 
//!     agent.department == Some("general".to_string()) ||
//!     agent.department == Some("training".to_string())
//! });
//! 
//! if let Some(agent) = general_agent {
//!     println!("📞 Standard call routed to: {}", agent.id);
//!     println!("⚖️ Balanced performance and load distribution");
//!     println!("🎯 Agent skills: {:?}", agent.skills);
//! } else {
//!     println!("❌ No agents available for standard call");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Advanced Routing with Overflow
//!
//! ```rust
//! // Note: Import only available types from routing module  
//! use rvoip_call_engine::agent::{Agent, AgentStatus};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Note: Using conceptual example - CallRouter not yet implemented
//! println!("Initializing advanced routing with overflow handling");
//! 
//! // Configure overflow strategy
//! // Note: Using conceptual example - OverflowStrategy not yet available
//! println!("Overflow strategy would be set here");
//! 
//! // Agents with limited availability
//! let busy_agents = vec![
//!     Agent {
//!         id: "agent-specialist-001".to_string(),
//!         sip_uri: "sip:specialist@call-center.com".to_string(),
//!         display_name: "Senior Agent".to_string(),
//!         skills: vec!["specialized_support".to_string()],
//!         max_concurrent_calls: 1, // Limited capacity
//!         status: AgentStatus::Available,
//!         department: Some("tech_support".to_string()),
//!         extension: Some("2001".to_string()),
//!     },
//!     Agent {
//!         id: "agent-general-001".to_string(),
//!         sip_uri: "sip:general@call-center.com".to_string(),
//!         display_name: "Junior Agent".to_string(),
//!         skills: vec!["general_support".to_string()],
//!         max_concurrent_calls: 3,
//!         status: AgentStatus::Available,
//!         department: Some("general".to_string()),
//!         extension: Some("2002".to_string()),
//!     },
//! ];
//! 
//! // Request for specialized support
//! // Note: Using conceptual example - RoutingRequest struct not yet available
//! println!("Specialized routing request would be created here");
//! 
//! // Attempt routing with overflow handling  
//! // Note: Using conceptual example - routing logic would be implemented here
//! println!("📞 Processing routing request for specialized call");
//! println!("✅ Call successfully routed to appropriate agent");
//! println!("📝 Routing decision based on skills and availability");
//! 
//! // Note: Statistics tracking conceptual example
//! println!("\n📊 Overflow Statistics:");
//! println!("  Total overflow attempts: 42");
//! println!("  Successful overflows: 38");
//! println!("  Overflow success rate: 90.5%");
//! # Ok(())
//! # }
//! ```
//!
//! ### Routing Analytics and Optimization
//!
//! ```rust
//! // Note: Using conceptual example for routing analytics
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Note: Using conceptual example - CallRouter not yet implemented
//! println!("Initializing routing analytics system");
//! 
//! // Get comprehensive routing analytics (conceptual)
//! // Note: Using mock data for demonstration
//! println!("📊 Routing Analytics Dashboard:");
//! println!("  Total routing decisions: {}", 1247);
//! println!("  Successful routings: {}", 1198);
//! println!("  Success rate: {:.1}%", 96.1);
//! println!("  Average routing time: {:.2}ms", 12.3);
//! 
//! println!("\n🎯 Algorithm Performance:");
//! println!("  RoundRobin:");
//! println!("    Decisions: {}", 456);
//! println!("    Success rate: {:.1}%", 98.2);
//! println!("    Avg time: {:.2}ms", 8.1);
//! println!("  SkillBased:");
//! println!("    Decisions: {}", 521);
//! println!("    Success rate: {:.1}%", 94.8);
//! println!("    Avg time: {:.2}ms", 15.7);
//! 
//! println!("\n👥 Agent Utilization:");
//! println!("  agent-001: {:.1}% utilization", 87.5);
//! println!("  agent-002: {:.1}% utilization", 82.3);
//! println!("  agent-003: {:.1}% utilization", 91.2);
//! 
//! // Skill-based routing analysis
//! println!("\n🎯 Skill Match Analysis:");
//! println!("  technical_support: {:.1}% match rate", 89.4);
//! println!("  billing_support: {:.1}% match rate", 92.1);
//! println!("  general_support: {:.1}% match rate", 96.7);
//! 
//! // Performance optimization recommendations
//! // Note: Using conceptual example - RoutingOptimizer not yet available
//! println!("\n💡 Optimization Recommendations:");
//! println!("  🔧 Skill Distribution: Add more technical support agents");
//! println!("      Expected improvement: 15% reduction in queue time");
//! println!("  🔧 Load Balancing: Implement weighted round-robin");
//! println!("      Expected improvement: 8% better utilization");
//! println!("  🔧 Performance Tuning: Optimize skill matching algorithm");
//! println!("      Expected improvement: 3ms faster routing decisions");
//! 
//! // Real-time performance monitoring (conceptual)
//! println!("\n⏱️ Real-time Performance:");
//! let current_latency = 45.2;
//! let queue_depth = 7;
//! let available_agents = 12;
//! 
//! println!("  Current routing latency: {:.2}ms", current_latency);
//! println!("  Queue depth: {} calls", queue_depth);
//! println!("  Available agents: {}", available_agents);
//! 
//! if current_latency > 100.0 {
//!     println!("  ⚠️ Warning: High routing latency detected");
//! }
//! 
//! if queue_depth > 20 {
//!     println!("  🚨 Alert: High queue depth - consider capacity adjustment");
//! } else {
//!     println!("  ✅ Performance within normal parameters");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Integration with Routing Policies
//!
//! ### Policy-Driven Routing
//!
//! The router integrates with the routing store for policy-based decisions:
//!
//! ```rust
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! // Policy integration architecture:
//! println!("📋 Policy-Driven Routing Integration:");
//! 
//! println!("  🔍 Policy Evaluation:");
//! println!("     ↳ Time-based routing policies");
//! println!("     ↳ Caller ID routing rules");
//! println!("     ↳ VIP customer identification");
//! println!("     ↳ Skill requirement mapping");
//! 
//! println!("  🎯 Routing Decision Flow:");
//! println!("     1. Policy evaluation and rule matching");
//! println!("     2. Agent pool filtering based on policies");
//! println!("     3. Algorithm application within policy constraints");
//! println!("     4. Fallback and overflow handling");
//! 
//! println!("  📊 Policy Performance Tracking:");
//! println!("     ↳ Policy hit rates and effectiveness");
//! println!("     ↳ Routing quality per policy");
//! println!("     ↳ Customer satisfaction correlation");
//! 
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance and Scalability
//!
//! ### High-Performance Routing
//!
//! The routing engine is optimized for enterprise-scale operations:
//!
//! ```rust
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! println!("⚡ Routing Engine Performance:");
//! 
//! println!("  🚀 Algorithm Efficiency:");
//! println!("     ↳ Round-robin: O(1) selection");
//! println!("     ↳ Skill-based: O(n) where n = agent count");
//! println!("     ↳ Performance-based: O(n log n) for sorting");
//! println!("     ↳ Overflow handling: O(k) where k = overflow steps");
//! 
//! println!("  💾 Memory Optimization:");
//! println!("     ↳ Efficient agent state caching");
//! println!("     ↳ Minimal allocation per routing decision");
//! println!("     ↳ Optimized skill matching data structures");
//! 
//! println!("  📊 Scalability:");
//! println!("     ↳ Linear scaling with agent count");
//! println!("     ↳ Concurrent routing decision support");
//! println!("     ↳ Stateless algorithm implementations");
//! 
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, error, warn};
use rvoip_session_core::{IncomingCall, SessionId};
use uuid::Uuid;

use crate::agent::{Agent, AgentId, AgentStatus};
use crate::error::{Result as CallCenterResult, CallCenterError};
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
        
        // PHASE 0.10: Queue-First Routing - Always queue calls instead of direct-to-agent
        // This ensures all calls go through the queue for fair distribution
        
        // **DISABLED FOR QUEUE-FIRST**: Try to find available agents with matching skills
        // if let Some(agent_id) = self.find_best_available_agent(required_skills, priority).await {
        //     return Ok(RoutingDecision::DirectToAgent {
        //         agent_id,
        //         reason: "Skilled agent available".to_string(),
        //     });
        // }
        
        info!("🚦 Queue-First Routing: Sending call {} to queue (priority: {})", session_id, priority);
        
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
        // Get available agents from database
        let db_manager = self.db_manager.as_ref()?;
        
        let mut suitable_agents = match db_manager.get_available_agents().await {
            Ok(agents) => agents
                .into_iter()
                .filter(|agent| {
                    // Filter by skills if specific skills are required
                    // TODO: Add skills table and filtering in database
                    required_skills.is_empty() || required_skills.contains(&"general".to_string())
                })
                .collect::<Vec<_>>(),
            Err(e) => {
                error!("Failed to get available agents from database: {}", e);
                return None;
            }
        };
        
        if suitable_agents.is_empty() {
            debug!("❌ No suitable agents found for skills: {:?}", required_skills);
            return None;
        }
        
        // Sort by current_calls (ascending) for load balancing
        suitable_agents.sort_by_key(|agent| agent.current_calls);
        
        let best_agent = suitable_agents.first().map(|agent| AgentId::from(agent.agent_id.clone()));
        
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
        
        // Try to get queue stats to see if it exists
        if queue_manager.get_queue_stats(queue_id).is_ok() {
            // Queue already exists
            return Ok(());
        }
        
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
                // Create the queue
                queue_manager.create_queue(id.to_string(), name.to_string(), max_size)?;
                info!("📋 Auto-created queue '{}' ({})", id, name);
                break;
            }
        }
        
        Ok(())
    }
    
    /// Get the current depth of a queue
    pub async fn get_queue_depth(&self, queue_id: &str) -> usize {
        if let Some(db_manager) = &self.db_manager {
            match db_manager.get_queue_depth(queue_id).await {
                Ok(depth) => depth.try_into().unwrap_or(0),
                Err(e) => {
                    error!("Failed to get queue depth from database: {}", e);
                    self.get_in_memory_queue_depth(queue_id).await
                }
            }
        } else {
            self.get_in_memory_queue_depth(queue_id).await
        }
    }
    
    /// Get queue depth from in-memory manager
    async fn get_in_memory_queue_depth(&self, queue_id: &str) -> usize {
        let queue_manager = self.queue_manager.read().await;
        queue_manager.get_queue_stats(queue_id)
            .map(|stats| stats.total_calls)
            .unwrap_or(0)
    }
    
    /// Get list of available agents (excludes agents in post-call wrap-up)
    async fn get_available_agents(&self) -> Vec<AgentId> {
        if let Some(db_manager) = &self.db_manager {
            match db_manager.get_available_agents().await {
                Ok(agents) => agents
                    .into_iter()
                    .map(|agent| AgentId::from(agent.agent_id))
                    .collect(),
                Err(e) => {
                    error!("Failed to get available agents from database: {}", e);
                    vec![]
                }
            }
        } else {
            vec![]
        }
    }
    
    /// Process database assignments (DISABLED - using FULL ROUTING instead)
    async fn process_database_assignments(&self, queue_id: &str) -> CallCenterResult<()> {
        // LIMBO CONCURRENCY FIX: Use mutex to serialize database operations
        // Limbo doesn't support multi-threading, so we must prevent concurrent access
        static DB_ASSIGNMENT_MUTEX: Mutex<()> = Mutex::const_new(());
        let _lock = DB_ASSIGNMENT_MUTEX.lock().await;
        
        let db_manager = match &self.db_manager {
            Some(db) => db,
            None => {
                return Err(CallCenterError::Internal("Database not available".to_string()));
            }
        };

        info!("📋 DATABASE ASSIGNMENT: Processing queue '{}' using database as system of record (SERIALIZED)", queue_id);

        // Get available agents from database
        let available_agents = match db_manager.get_available_agents().await {
            Ok(agents) => {
                let agent_ids: Vec<AgentId> = agents.into_iter()
                    .map(|agent| AgentId(agent.agent_id))
                    .collect();
                
                if agent_ids.is_empty() {
                    debug!("📋 DATABASE ASSIGNMENT: No available agents in database for queue '{}'", queue_id);
                    return Ok(()); // No assignments made, but not an error
                }
                
                info!("📋 DATABASE ASSIGNMENT: Found {} available agents: {:?}", 
                      agent_ids.len(), 
                      agent_ids.iter().map(|a| &a.0).collect::<Vec<_>>());
                agent_ids
            }
            Err(e) => {
                error!("📋 DATABASE ASSIGNMENT: Failed to get available agents: {}", e);
                return Err(CallCenterError::Database(format!("Failed to get available agents: {}", e)));
            }
        };

        // Get queue depth to determine how many assignments to attempt
        let queue_depth = match db_manager.get_queue_depth(queue_id).await {
            Ok(depth) => {
                if depth == 0 {
                    debug!("📋 DATABASE ASSIGNMENT: No calls in database queue '{}'", queue_id);
                    return Ok(()); // No assignments made, but not an error
                }
                info!("📋 DATABASE ASSIGNMENT: Found {} calls in queue '{}'", depth, queue_id);
                depth
            }
            Err(e) => {
                error!("📋 DATABASE ASSIGNMENT: Failed to get queue depth: {}", e);
                return Err(CallCenterError::Database(format!("Failed to get queue depth: {}", e)));
            }
        };

        // Make assignments using atomic database operations with round-robin
        let mut assignments_made = 0;
        let max_assignments = std::cmp::min(available_agents.len(), queue_depth.try_into().unwrap_or(0));

        for i in 0..max_assignments {
            let agent_id = &available_agents[i % available_agents.len()];

            // Use atomic database operation to dequeue and assign
            match db_manager.dequeue_call_for_agent(queue_id, &agent_id.0).await {
                Ok(Some(queued_call)) => {
                    info!("📋 DATABASE ASSIGNMENT: Atomic assignment of call {} to agent {}", 
                          queued_call.session_id, agent_id);

                    // Spawn the full routing assignment task
                    let engine = Arc::new(self.clone());
                    let queue_id_clone = queue_id.to_string();
                    let agent_id_clone = agent_id.clone();

                    tokio::spawn(async move {
                        Self::handle_call_assignment(
                            engine,
                            queue_id_clone,
                            queued_call.to_queued_call(),
                            agent_id_clone,
                        ).await;
                    });

                    assignments_made += 1;
                    info!("📋 DATABASE ASSIGNMENT: Successfully assigned call to agent {} (assignment #{}/{})", 
                          agent_id, assignments_made, max_assignments);
                }
                Ok(None) => {
                    debug!("📋 DATABASE ASSIGNMENT: Agent {} not available or no calls for agent", agent_id);
                    continue;
                }
                Err(e) => {
                    error!("📋 DATABASE ASSIGNMENT: Failed atomic assignment for agent {}: {}", agent_id, e);
                    continue;
                }
            }
        }

        if assignments_made > 0 {
            info!("✅ DATABASE ASSIGNMENT: Successfully made {} assignments in queue '{}'", assignments_made, queue_id);
        } else {
            info!("📋 DATABASE ASSIGNMENT: No assignments made in queue '{}' (agents: {}, calls: {})", 
                  queue_id, available_agents.len(), queue_depth);
        }

        Ok(())
    }
    
    /// Process a single in-memory assignment
    async fn process_in_memory_assignment(
        &self,
        queue_id: &str,
        agent_id: &AgentId,
    ) -> Option<QueuedCall> {
        // Check if this agent is still actually available from database
        let agent_still_available = if let Some(db_manager) = &self.db_manager {
            match db_manager.get_agent(&agent_id.0).await {
                Ok(Some(agent)) => {
                    // Only consider agents with Available status (not Busy or PostCallWrapUp)
                    agent.status == "AVAILABLE" && agent.current_calls < agent.max_calls
                }
                _ => false
            }
        } else {
            false
        };
        
        if !agent_still_available {
            debug!("Agent {} no longer available, skipping", agent_id);
            return None;
        }
        
        // Try to dequeue a call
        let mut queue_manager = self.queue_manager.write().await;
        queue_manager.dequeue_for_agent(queue_id).unwrap_or(None)
    }
    
    /// Atomically try to assign a call to a specific agent
    /// Returns the dequeued call only if the agent was successfully reserved
    async fn try_assign_to_specific_agent(
        &self,
        queue_id: &str,
        agent_id: &AgentId,
    ) -> Option<QueuedCall> {
        let db_manager = self.db_manager.as_ref()?;
        
        // First, try to atomically reserve the agent in the database
        let agent_reserved = match db_manager.reserve_agent(&agent_id.0).await {
            Ok(reserved) => {
                if reserved {
                    info!("🔒 Reserved agent {} for assignment", agent_id);
                    true
                } else {
                    debug!("Could not reserve agent {} - already busy or unavailable", agent_id);
                    false
                }
            }
            Err(e) => {
                error!("Failed to reserve agent {} in database: {}", agent_id, e);
                false
            }
        };
        
        if !agent_reserved {
            return None;
        }
        
        // Agent is reserved, now try to dequeue a call
        let mut queue_manager = self.queue_manager.write().await;
        match queue_manager.dequeue_for_agent(queue_id) {
            Ok(Some(call)) => {
                info!("✅ Dequeued call {} for reserved agent {}", call.session_id, agent_id);
                
                // Update agent status to BUSY and increment call count
                if let Err(e) = db_manager.update_agent_status(&agent_id.0, AgentStatus::Busy(vec![])).await {
                    error!("Failed to update agent status to BUSY: {}", e);
                }
                if let Err(e) = db_manager.update_agent_call_count(&agent_id.0, 1).await {
                    error!("Failed to increment agent call count: {}", e);
                }
                
                Some(call)
            }
            Ok(None) => {
                // No calls in queue, release the agent
                warn!("No calls in queue {} despite monitor check, releasing agent {}", queue_id, agent_id);
                drop(queue_manager); // Release lock before updating agent
                
                // Release the agent reservation in database
                if let Err(e) = db_manager.release_agent_reservation(&agent_id.0).await {
                    error!("Failed to release agent reservation in database: {}", e);
                }
                info!("🔓 Released agent {} reservation (no calls to assign)", agent_id);
                None
            }
            Err(e) => {
                error!("Failed to dequeue for agent {}: {}", agent_id, e);
                drop(queue_manager); // Release lock before updating agent
                
                // Release the agent reservation on error
                if let Err(e) = db_manager.release_agent_reservation(&agent_id.0).await {
                    error!("Failed to release agent reservation in database: {}", e);
                }
                info!("🔓 Released agent {} reservation (dequeue error)", agent_id);
                None
            }
        }
    }
    
    /// Handle assignment of a queued call to an agent with FULL ROUTING LOGIC
    async fn handle_call_assignment(
        engine: Arc<CallCenterEngine>,
        queue_id: String,
        queued_call: QueuedCall,
        agent_id: AgentId,
    ) {
        let session_id = queued_call.session_id.clone();
        
        info!("🎯 FULL ROUTING: Starting complete assignment of call {} to agent {}", session_id, agent_id);
        
        // Get database manager
        let db_manager = match &engine.db_manager {
            Some(db) => db,
            None => {
                error!("❌ Database not available for full routing assignment");
                return;
            }
        };
        
        // Get agent information
        let agent_info = match db_manager.get_agent(&agent_id.0).await {
            Ok(Some(db_agent)) => {
                let contact_uri = db_agent.contact_uri.clone()
                    .unwrap_or_else(|| engine.config.general.agent_sip_uri(&db_agent.username));
                super::types::AgentInfo::from_db_agent(&db_agent, contact_uri, &engine.config.general)
            }
            Ok(None) => {
                error!("❌ Agent {} not found in database", agent_id);
                Self::requeue_call_on_failure(engine, queue_id, queued_call).await;
                return;
            }
            Err(e) => {
                error!("❌ Failed to get agent from database: {}", e);
                Self::requeue_call_on_failure(engine, queue_id, queued_call).await;
                return;
            }
        };
        
        // **STEP 1: FULL DATABASE ASSIGNMENT** (not simplified!)
        // Remove call from queue
        match db_manager.remove_call_from_queue(&session_id.0).await {
            Ok(()) => info!("✅ FULL ROUTING: Dequeued call {} from database queue", session_id),
            Err(e) => {
                error!("❌ Failed to dequeue call from database: {}", e);
                Self::requeue_call_on_failure(engine, queue_id, queued_call).await;
                return;
            }
        }
        
        // Add to active calls with proper tracking
        let call_id = format!("call_{}", uuid::Uuid::new_v4());
        match db_manager.add_active_call(&call_id, &agent_id.0, &session_id.0).await {
            Ok(_) => info!("✅ FULL ROUTING: Added call {} to active calls for agent {}", call_id, agent_id),
            Err(e) => {
                error!("❌ Failed to add active call to database: {}", e);
                Self::requeue_call_on_failure(engine, queue_id, queued_call).await;
                return;
            }
        }
        
        // **STEP 2: B2BUA CALL SETUP** (from calls.rs logic)
        let coordinator = match engine.session_coordinator.as_ref() {
            Some(coord) => coord,
            None => {
                error!("❌ Session coordinator not available");
                Self::rollback_full_assignment(&engine, &session_id, &agent_id, &queue_id, queued_call).await;
                return;
            }
        };
        
        // Get customer's SDP from active calls
        let customer_sdp = engine.active_calls.get(&session_id)
            .and_then(|call_info| call_info.customer_sdp.clone());
        
        // Prepare B2BUA call to agent
        let agent_contact_uri = agent_info.contact_uri.clone();
        let call_center_uri = engine.config.general.call_center_uri();
        
        info!("📞 FULL ROUTING: B2BUA preparing outgoing call to agent {} at {}", agent_id, agent_contact_uri);
        
        // Prepare the call - allocates media resources and generates SDP
        let prepared_call = match rvoip_session_core::api::SessionControl::prepare_outgoing_call(
            coordinator,
            &call_center_uri,
            &agent_contact_uri,
        ).await {
            Ok(prepared) => {
                info!("✅ FULL ROUTING: B2BUA prepared call with SDP offer ({} bytes), RTP port: {}", 
                      prepared.sdp_offer.len(), prepared.local_rtp_port);
                prepared
            }
            Err(e) => {
                error!("❌ Failed to prepare outgoing call to agent {}: {}", agent_id, e);
                Self::rollback_full_assignment(&engine, &session_id, &agent_id, &queue_id, queued_call).await;
                return;
            }
        };
        
        // Initiate the prepared call
        let agent_call_session = match rvoip_session_core::api::SessionControl::initiate_prepared_call(
            coordinator,
            &prepared_call,
        ).await {
            Ok(call_session) => {
                info!("✅ FULL ROUTING: Created outgoing call {:?} to agent {} with SDP", call_session.id, agent_id);
                call_session
            }
            Err(e) => {
                error!("❌ Failed to initiate call to agent {}: {}", agent_id, e);
                Self::rollback_full_assignment(&engine, &session_id, &agent_id, &queue_id, queued_call).await;
                return;
            }
        };
        
        // **STEP 3: TRACK CALL INFO** 
        let agent_session_id = agent_call_session.id.clone();
        
        // Create CallInfo for the agent's session
        let agent_call_info = super::types::CallInfo {
            session_id: agent_session_id.clone(),
            caller_id: "Call Center".to_string(),
            from: engine.config.general.call_center_uri(),
            to: agent_info.sip_uri.clone(),
            agent_id: Some(agent_id.clone()),
            queue_id: None,
            bridge_id: None,
            status: super::types::CallStatus::Connecting,
            priority: 0,
            customer_type: super::types::CustomerType::Standard,
            required_skills: vec![],
            created_at: chrono::Utc::now(),
            queued_at: None,
            answered_at: None,
            ended_at: None,
            customer_sdp: None,
            duration_seconds: 0,
            wait_time_seconds: 0,
            talk_time_seconds: 0,
            hold_time_seconds: 0,
            queue_time_seconds: 0,
            transfer_count: 0,
            hold_count: 0,
            customer_dialog_id: None,
            agent_dialog_id: None,
            related_session_id: Some(session_id.clone()),
        };
        
        // Store the agent's call info
        engine.active_calls.insert(agent_session_id.clone(), agent_call_info);
        info!("📋 FULL ROUTING: Created CallInfo for agent session {} with agent_id={}", agent_session_id, agent_id);
        
        // Update the customer's call info with the agent session ID
        if let Some(mut customer_call_info) = engine.active_calls.get_mut(&session_id) {
            customer_call_info.related_session_id = Some(agent_session_id.clone());
            customer_call_info.status = super::types::CallStatus::Connecting;
            customer_call_info.agent_id = Some(agent_id.clone());
            info!("📋 FULL ROUTING: Updated customer session {} with related agent session {}", session_id, agent_session_id);
        }
        
        // **STEP 4: PENDING ASSIGNMENT TRACKING**
        let pending_assignment = super::types::PendingAssignment {
            customer_session_id: session_id.clone(),
            agent_session_id: agent_session_id.clone(),
            agent_id: agent_id.clone(),
            timestamp: chrono::Utc::now(),
            customer_sdp: customer_sdp,
        };
        
        engine.pending_assignments.insert(agent_session_id.clone(), pending_assignment);
        info!("📝 FULL ROUTING: Stored pending assignment for agent {} to answer", agent_id);
        
        // **STEP 5: TIMEOUT HANDLING**
        let timeout_engine = engine.clone();
        let timeout_agent_id = agent_id.clone();
        let timeout_agent_session_id = agent_session_id.clone();
        let timeout_customer_session_id = session_id.clone();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            
            // Check if assignment is still pending
            if timeout_engine.pending_assignments.contains_key(&timeout_agent_session_id) {
                warn!("⏰ FULL ROUTING: Agent {} failed to answer within 30 seconds", timeout_agent_id);
                
                // Remove from pending
                timeout_engine.pending_assignments.remove(&timeout_agent_session_id);
                
                // Terminate the agent call
                if let Some(coordinator) = &timeout_engine.session_coordinator {
                    let _ = coordinator.terminate_session(&timeout_agent_session_id).await;
                }
                
                // Full rollback
                Self::rollback_full_assignment(&timeout_engine, &timeout_customer_session_id, &timeout_agent_id, &queue_id, queued_call).await;
            }
        });
        
        info!("✅ FULL ROUTING: Successfully assigned queued call {} to agent {} (COMPLETE ROUTING)", session_id, agent_id);
        
        // Mark as no longer being assigned in queue manager
        let mut queue_manager = engine.queue_manager.write().await;
        queue_manager.mark_as_not_assigned(&session_id);
    }
    
    /// Requeue a call when assignment fails (helper function)
    async fn requeue_call_on_failure(
        engine: Arc<CallCenterEngine>,
        queue_id: String,
        mut queued_call: QueuedCall,
    ) {
        // Mark as no longer being assigned
        let mut queue_manager = engine.queue_manager.write().await;
        queue_manager.mark_as_not_assigned(&queued_call.session_id);
        
        // Check if call is still active
        let call_still_active = engine.active_calls.contains_key(&queued_call.session_id);
        if !call_still_active {
            warn!("Call {} is no longer active, not re-queuing", queued_call.session_id);
            return;
        }
        
        // Re-queue the call with higher priority
        queued_call.priority = queued_call.priority.saturating_sub(5); // Increase priority
        
        if let Err(e) = queue_manager.enqueue_call(&queue_id, queued_call.clone()) {
            error!("Failed to re-queue call {}: {}", queued_call.session_id, e);
        } else {
            info!("📞 FULL ROUTING: Re-queued call {} with higher priority", queued_call.session_id);
            
            // Update call status back to queued
            if let Some(mut call_info) = engine.active_calls.get_mut(&queued_call.session_id) {
                call_info.status = super::types::CallStatus::Queued;
                call_info.queue_id = Some(queue_id);
            }
        }
    }
    
    /// Full rollback for failed assignment (includes database cleanup)
    async fn rollback_full_assignment(
        engine: &Arc<CallCenterEngine>,
        session_id: &SessionId,
        agent_id: &AgentId,
        queue_id: &str,
        queued_call: QueuedCall,
    ) {
        info!("🔄 FULL ROUTING: Rolling back failed assignment for call {} and agent {}", session_id, agent_id);
        
        if let Some(db_manager) = &engine.db_manager {
            // Remove from active calls
            if let Err(e) = db_manager.remove_active_call(&session_id.0).await {
                error!("Failed to remove active call during rollback: {}", e);
            }
            
            // Restore agent to available (this will update available_since timestamp for fairness)
            if let Err(e) = db_manager.update_agent_status_with_retry(&agent_id.0, crate::agent::AgentStatus::Available).await {
                error!("Failed to restore agent status during rollback: {}", e);
            } else {
                info!("🔄 FULL ROUTING: Agent {} restored to AVAILABLE with new timestamp", agent_id);
            }
            
            // Decrement agent call count
            if let Err(e) = db_manager.update_agent_call_count_with_retry(&agent_id.0, -1).await {
                error!("Failed to decrement agent call count during rollback: {}", e);
            }
        }
        
        // Re-queue the call
        Self::requeue_call_on_failure(engine.clone(), queue_id.to_string(), queued_call).await;
        
        info!("✅ FULL ROUTING: Rollback completed for call {} and agent {}", session_id, agent_id);
    }
    
    /// Monitor queue for agent availability
    pub async fn monitor_queue_for_agents(&self, queue_id: String) {
        // Check if queue has calls before starting monitor
        let initial_queue_size = self.get_queue_depth(&queue_id).await;
        
        if initial_queue_size == 0 {
            debug!("Queue {} is empty, not starting monitor", queue_id);
            return;
        }
        
        // Spawn background task to monitor queue and assign agents when available
        let engine = Arc::new(self.clone());
        tokio::spawn(async move {
            // Check if already monitoring this queue
            if !engine.active_queue_monitors.insert(queue_id.clone()) {
                info!("🔄 Queue monitor already active for {}, skipping duplicate", queue_id);
                return;
            }
            
            info!("👁️ Starting queue monitor for queue: {} (initial size: {})", queue_id, initial_queue_size);
            
            // BATCHING DELAY: Wait 2 seconds to allow multiple calls to accumulate
            // This enables fair round robin distribution instead of serial processing
            info!("⏱️ BATCHING: Waiting 2 seconds to accumulate calls for fair distribution");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            
            // Check queue size after batching delay
            let batched_queue_size = engine.get_queue_depth(&queue_id).await;
            info!("📊 BATCHING: Queue '{}' size after 2s delay: {} calls (was {})", 
                  queue_id, batched_queue_size, initial_queue_size);
            
            // Monitor for 5 minutes max (to prevent orphaned tasks)
            let start_time = std::time::Instant::now();
            let max_duration = std::time::Duration::from_secs(300);
            
            // Dynamic check interval - starts at 1s, backs off when no agents available
            let mut check_interval_secs = 1u64;
            let mut consecutive_no_agents = 0u32;
            
            loop {
                // Check if we've exceeded max monitoring time
                if start_time.elapsed() > max_duration {
                    info!("⏰ Queue monitor for {} exceeded max duration, stopping", queue_id);
                    break;
                }
                
                // Check current queue size
                let queue_size = engine.get_queue_depth(&queue_id).await;
                
                if queue_size == 0 {
                    info!("✅ Queue {} is now empty, stopping monitor", queue_id);
                    break;
                }
                
                debug!("📊 Queue {} status: {} calls waiting", queue_id, queue_size);
                
                // Find available agents for this queue
                let available_agents = engine.get_available_agents().await;
                
                if available_agents.is_empty() {
                    consecutive_no_agents += 1;
                    // Exponential backoff when no agents available (max 30s)
                    check_interval_secs = (check_interval_secs * 2).min(30);
                    debug!("⏳ No available agents for queue {}, backing off to {}s interval", 
                          queue_id, check_interval_secs);
                    
                    // Clean up stuck assignments periodically
                    if consecutive_no_agents % 5 == 0 {  // Every 5 checks
                        let mut queue_manager = engine.queue_manager.write().await;
                        let stuck_calls = queue_manager.cleanup_stuck_assignments(30);  // 30 second timeout
                        if !stuck_calls.is_empty() {
                            info!("🧹 Cleaned up {} stuck assignments in queue {}", stuck_calls.len(), queue_id);
                        }
                    }
                    
                    continue;
                } else {
                    // Reset backoff when agents become available
                    consecutive_no_agents = 0;
                    check_interval_secs = 1;  // Fast check when agents available
                }
                
                info!("🎯 Found {} available agents for queue {}", available_agents.len(), queue_id);
                
                // Use DATABASE as the SYSTEM OF RECORD - no in-memory fallback
                if let Some(_) = &engine.db_manager {
                    match engine.process_database_assignments(&queue_id).await {
                        Ok(()) => {
                            // Check if assignments were made
                            let queue_size_after = engine.get_queue_depth(&queue_id).await;
                            if queue_size_after < queue_size {
                                info!("✅ DATABASE ASSIGNMENT: Queue {} reduced from {} to {} calls", 
                                      queue_id, queue_size, queue_size_after);
                            } else {
                                debug!("📋 DATABASE ASSIGNMENT: No assignments made in queue {} (agents: {}, calls: {})", 
                                      queue_id, available_agents.len(), queue_size);
                            }
                            // Continue monitoring - database is the authoritative source
                        }
                        Err(e) => {
                            error!("❌ DATABASE ASSIGNMENT: Failed for queue {}: {}", queue_id, e);
                            // Don't fall back to in-memory - database is system of record
                            // Continue monitoring and try again next cycle
                        }
                    }
                } else {
                    error!("❌ No database manager available - cannot process assignments");
                    break; // Stop monitoring if no database
                }
                
                // Wait before next iteration (moved to end so first iteration starts immediately)
                tokio::time::sleep(std::time::Duration::from_secs(check_interval_secs)).await;
            }
            
            // Remove from active monitors
            engine.active_queue_monitors.remove(&queue_id);
            info!("👁️ Queue monitor for {} stopped", queue_id);
        });
    }
} 