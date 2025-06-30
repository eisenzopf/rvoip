//! # Call Routing Engine Module
//!
//! This module provides the intelligent call routing system for the call center,
//! including routing strategies, skill-based routing, load balancing, and advanced
//! routing policies. The routing engine determines how incoming calls are distributed
//! among available agents or queued for later processing.
//!
//! ## Architecture
//!
//! The routing system follows a decision-tree architecture:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    Incoming Call                            │
//! │  (Session ID, caller info, urgency, skills required)       │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │
//! ┌─────────────────────────▼───────────────────────────────────┐
//! │                  Routing Engine                             │
//! │  - Call classification                                      │
//! │  - Skill matching                                           │
//! │  - Agent availability                                       │
//! │  - Load balancing                                           │
//! │  - Business rules                                           │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │
//!           ┌───────────────┼───────────────┐
//!           │               │               │
//! ┌─────────▼─────────┐ ┌─────▼─────┐ ┌─────▼──────┐
//! │ Direct to Agent   │ │   Queue   │ │   Reject   │
//! │                   │ │           │ │            │
//! │ • Skill match     │ │ • Priority│ │ • Overload │
//! │ • Available now   │ │ • Wait    │ │ • No match │
//! │ • Load balanced   │ │ • Overflow│ │ • Policy   │
//! └───────────────────┘ └───────────┘ └────────────┘
//! ```
//!
//! ## Core Features
//!
//! ### **Skill-Based Routing**
//! - Match agent skills with call requirements
//! - Support for required vs. preferred skills
//! - Hierarchical skill levels and proficiency
//! - Dynamic skill updates
//!
//! ### **Load Balancing**
//! - Even distribution across available agents
//! - Workload-aware assignment
//! - Agent capacity considerations
//! - Performance-based routing
//!
//! ### **Business Rules**
//! - Time-based routing (business hours, after hours)
//! - Customer tier-based prioritization
//! - Geographic routing preferences
//! - Escalation policies
//!
//! ### **Intelligent Queuing**
//! - Overflow to appropriate queues
//! - Priority-based queue selection
//! - Queue capacity awareness
//! - SLA-driven queue routing
//!
//! ## Routing Strategies
//!
//! The routing engine supports multiple strategies that can be combined:
//!
//! ### **Round Robin**
//! - Distributes calls evenly among agents
//! - Simple and predictable
//! - Good for training environments
//!
//! ### **Skill-Based**
//! - Routes based on agent skills and call requirements
//! - Optimizes for expertise matching
//! - Improves first-call resolution
//!
//! ### **Least Busy**
//! - Routes to agents with lowest current workload
//! - Balances agent utilization
//! - Prevents overload situations
//!
//! ### **Performance-Based**
//! - Routes to highest performing agents
//! - Optimizes for customer satisfaction
//! - Incentivizes agent performance
//!
//! ## Quick Start
//!
//! ### Basic Routing Setup
//!
//! ```rust
//! use rvoip_call_engine::routing::{RoutingEngine, RoutingDecision};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create routing engine with default policies
//! let routing_engine = RoutingEngine::new();
//! 
//! // Route an incoming call
//! let decision = routing_engine.route_call("incoming-call-info").await?;
//! 
//! match decision {
//!     RoutingDecision::DirectToAgent { agent_id } => {
//!         println!("Route directly to agent: {}", agent_id);
//!     }
//!     RoutingDecision::Queue { queue_id } => {
//!         println!("Queue call in: {}", queue_id);
//!     }
//!     RoutingDecision::Reject { reason } => {
//!         println!("Reject call: {}", reason);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Skill-Based Routing Example
//!
//! ```rust
//! use rvoip_call_engine::routing::SkillMatcher;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let skill_matcher = SkillMatcher::new();
//! 
//! // Define call requirements
//! let required_skills = vec!["english".to_string(), "billing".to_string()];
//! let preferred_skills = vec!["tier2".to_string()];
//! 
//! // Find best agent match
//! // let best_agent = skill_matcher.find_best_match(
//! //     &required_skills,
//! //     &preferred_skills,
//! //     &available_agents
//! // )?;
//! 
//! println!("Skill-based routing configured");
//! # Ok(())
//! # }
//! ```
//!
//! ### Advanced Routing Policies
//!
//! ```rust
//! use rvoip_call_engine::routing::RoutingPolicies;
//! 
//! # fn example() {
//! let mut policies = RoutingPolicies::new();
//! 
//! // Configure business hour routing
//! // policies.set_business_hours("09:00", "17:00", vec!["Mon", "Tue", "Wed", "Thu", "Fri"]);
//! 
//! // Configure VIP customer routing
//! // policies.set_vip_routing(true);
//! 
//! // Configure geographic preferences
//! // policies.set_geographic_routing(true);
//! 
//! println!("Advanced routing policies configured");
//! # }
//! ```
//!
//! ## Routing Decision Types
//!
//! The routing engine can make three types of decisions:
//!
//! ### **Direct to Agent**
//! - Agent is available and matches requirements
//! - Immediate call connection
//! - Best case scenario for customer experience
//!
//! ### **Queue for Later**
//! - No agents immediately available
//! - Queue based on call priority and type
//! - Estimated wait time provided to caller
//!
//! ### **Reject Call**
//! - System overload or capacity exceeded
//! - No suitable agents or queues available
//! - Business rule violation (e.g., after hours)
//!
//! ## Performance Optimization
//!
//! ### **Caching**
//! - Agent skill lookups cached for performance
//! - Routing decisions cached for similar calls
//! - Queue statistics cached for quick access
//!
//! ### **Load Balancing**
//! - Real-time agent workload monitoring
//! - Predictive load balancing based on call history
//! - Dynamic adjustment of routing weights
//!
//! ### **Metrics Collection**
//! - Routing decision timing and accuracy
//! - Agent performance tracking
//! - Customer satisfaction correlation
//!
//! ## Integration Points
//!
//! The routing engine integrates with:
//!
//! - **Agent Registry**: For availability and skills
//! - **Queue Manager**: For overflow and prioritization
//! - **Database**: For persistent routing rules
//! - **Monitoring**: For performance tracking
//! - **Configuration**: For business rules
//!
//! ## Advanced Features
//!
//! ### **Machine Learning Integration**
//! - Predictive routing based on historical data
//! - Customer satisfaction optimization
//! - Dynamic skill weight adjustment
//!
//! ### **Real-Time Adaptation**
//! - Automatic policy adjustment based on performance
//! - Dynamic queue thresholds
//! - Adaptive skill requirements
//!
//! ### **Geographic Routing**
//! - Route based on caller and agent location
//! - Time zone awareness
//! - Language preference matching
//!
//! ## Error Handling
//!
//! The routing system provides comprehensive error handling:
//!
//! ```rust
//! use rvoip_call_engine::routing::RoutingEngine;
//! use rvoip_call_engine::error::CallCenterError;
//! 
//! # async fn example() {
//! let routing_engine = RoutingEngine::new();
//! 
//! match routing_engine.route_call("call-info").await {
//!     Ok(decision) => println!("Routing decision: {:?}", decision),
//!     Err(CallCenterError::Routing(msg)) => println!("Routing error: {}", msg),
//!     Err(e) => println!("Other error: {}", e),
//! }
//! # }
//! ```
//!
//! ## Production Considerations
//!
//! ### **Scalability**
//! - Horizontal scaling for high call volumes
//! - Distributed routing decisions
//! - Load balancer integration
//!
//! ### **Reliability**
//! - Fallback routing strategies
//! - Circuit breaker patterns for external dependencies
//! - Graceful degradation under load
//!
//! ### **Monitoring**
//! - Real-time routing performance metrics
//! - Alert on routing failures or performance degradation
//! - A/B testing of routing strategies
//!
//! ## Modules
//!
//! - [`engine`]: Core routing engine and decision logic
//! - [`policies`]: Business rules and routing policies
//! - [`skills`]: Skill matching and agent capability assessment

pub mod engine;
pub mod policies;
pub mod skills;

pub use engine::{RoutingEngine, RoutingDecision};
pub use policies::{
    RoutingPolicies, TimeBasedRules as TimeBasedRule, BusinessHours, 
    GeographicRules as GeographicRule,
    SkillRequirement as PoliciesSkillRequirement, 
    SkillLevel as PoliciesSkillLevel, RoutingDecision as PoliciesRoutingDecision
};
pub use skills::{
    SkillMatcher, SkillHierarchy, SkillSubstitution, AgentSkillUpdate,
    MatchingConfig, MatchingAlgorithm, ScoringWeights,
    SkillRequirement, SkillLevel
};

// Re-export queue's CallContext since routing docs reference it
pub use crate::queue::CallContext; 