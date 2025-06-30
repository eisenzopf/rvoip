//! # Queue Policies and Call Prioritization
//!
//! This module provides sophisticated queue management policies for call prioritization,
//! queue assignment, and service level optimization. It enables intelligent queue
//! management based on customer priority, call type, business rules, and performance targets.

use std::collections::HashMap;
use chrono::{DateTime, Utc, Timelike, Datelike};
use serde::{Serialize, Deserialize};
use tracing::{info, debug, warn};

use crate::error::{CallCenterError, Result};

/// # Queue Policies for Call Prioritization and Management
///
/// The `QueuePolicies` system provides comprehensive queue management capabilities
/// including call prioritization, dynamic queue assignment, service level optimization,
/// and intelligent routing decisions. It ensures that calls are handled according to
/// business priorities and customer service level agreements.
///
/// ## Key Features
///
/// - **Dynamic Prioritization**: Real-time call priority adjustment based on multiple factors
/// - **Queue Assignment**: Intelligent assignment of calls to appropriate queues
/// - **Service Level Management**: SLA-based queue policies and escalation rules
/// - **Business Rules**: Configurable policies for complex business requirements
/// - **Performance Optimization**: Queue policies that optimize for key metrics
/// - **Time-Based Policies**: Different queue behaviors for business hours, peaks, etc.
///
/// ## Policy Types
///
/// ### Priority Policies
/// - Customer tier-based prioritization (VIP, Gold, Silver, Bronze)
/// - Call type priority (emergency, sales, support, billing)
/// - Time-sensitive priority adjustments
/// - Escalation-based priority increases
///
/// ### Queue Assignment Policies
/// - Skill-based queue routing
/// - Department-specific queues
/// - Geographic queue assignment
/// - Load balancing across queues
///
/// ### Service Level Policies
/// - SLA-based queue management
/// - Target response time policies
/// - Abandonment rate optimization
/// - Performance threshold management
///
/// ### Business Rule Policies
/// - Time-of-day queue routing
/// - Holiday and special event handling
/// - Capacity-based queue switching
/// - Custom business logic implementation
///
/// ## Examples
///
/// ### Basic Queue Policy Configuration
///
/// ```rust
/// use rvoip_call_engine::queue::{QueuePolicies, PriorityLevel};
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut policies = QueuePolicies::new();
/// 
/// // Configure customer tier priorities
/// policies.set_customer_priority("vip", PriorityLevel::Critical)?;
/// policies.set_customer_priority("gold", PriorityLevel::High)?;
/// policies.set_customer_priority("silver", PriorityLevel::Normal)?;
/// policies.set_customer_priority("bronze", PriorityLevel::Low)?;
/// 
/// // Configure call type priorities
/// policies.set_call_type_priority("emergency", PriorityLevel::Critical)?;
/// policies.set_call_type_priority("sales", PriorityLevel::High)?;
/// policies.set_call_type_priority("support", PriorityLevel::Normal)?;
/// policies.set_call_type_priority("billing", PriorityLevel::Low)?;
/// 
/// println!("Queue policies configured");
/// # Ok(())
/// # }
/// ```
///
/// ### Dynamic Priority Adjustment
///
/// ```rust
/// use rvoip_call_engine::queue::{QueuePolicies, CallContext};
/// use std::collections::HashMap;
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let policies = QueuePolicies::new();
/// 
/// // Create call context
/// let mut context = HashMap::new();
/// context.insert("customer_id".to_string(), "cust-12345".to_string());
/// context.insert("customer_tier".to_string(), "gold".to_string());
/// context.insert("call_type".to_string(), "support".to_string());
/// context.insert("wait_time".to_string(), "180".to_string()); // 3 minutes
/// 
/// let call_context = CallContext::new(context);
/// 
/// // Calculate dynamic priority
/// let priority = policies.calculate_priority(&call_context)?;
/// println!("Call priority: {:?} (score: {})", priority.level, priority.score);
/// 
/// // Check for priority escalation
/// if priority.should_escalate {
///     println!("⬆️ Call should be escalated due to wait time");
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Queue Assignment Logic
///
/// ```rust
/// use rvoip_call_engine::queue::{QueuePolicies, CallContext, QueueAssignment};
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let policies = QueuePolicies::new();
/// 
/// // Configure queue assignment rules
/// let mut context = std::collections::HashMap::new();
/// context.insert("customer_tier".to_string(), "vip".to_string());
/// context.insert("call_type".to_string(), "technical".to_string());
/// context.insert("customer_location".to_string(), "US".to_string());
/// 
/// let call_context = CallContext::new(context);
/// 
/// // Get queue assignment recommendation
/// let assignment = policies.assign_queue(&call_context)?;
/// 
/// match assignment {
///     QueueAssignment::Primary(queue_id) => {
///         println!("✅ Assigned to primary queue: {}", queue_id);
///     }
///     QueueAssignment::Secondary(queue_id, reason) => {
///         println!("🔄 Assigned to secondary queue: {} ({})", queue_id, reason);  
///     }
///     QueueAssignment::Reject(reason) => {
///         println!("❌ Call rejected: {}", reason);
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Service Level Policy Management
///
/// ```rust
/// use rvoip_call_engine::queue::{QueuePolicies, ServiceLevelTarget};
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut policies = QueuePolicies::new();
/// 
/// // Configure service level targets by queue
/// policies.set_service_level_target(
///     "vip_queue",
///     ServiceLevelTarget {
///         target_percentage: 95.0,  // 95% of calls
///         target_time_seconds: 15,  // Answered within 15 seconds
///         escalation_time_seconds: 60, // Escalate after 1 minute
///     }
/// )?;
/// 
/// policies.set_service_level_target(
///     "general_queue", 
///     ServiceLevelTarget {
///         target_percentage: 80.0,  // 80% of calls
///         target_time_seconds: 30,  // Answered within 30 seconds  
///         escalation_time_seconds: 120, // Escalate after 2 minutes
///     }
/// )?;
/// 
/// // Monitor service level compliance
/// let compliance = policies.check_service_level_compliance("vip_queue").await?;
/// println!("VIP queue SLA compliance: {:.1}%", compliance.current_percentage);
/// # Ok(())
/// # }
/// ```
///
/// ### Time-Based Policy Rules
///
/// ```rust
/// use rvoip_call_engine::queue::{QueuePolicies, TimeBasedRule, BusinessHours};
/// use chrono::NaiveTime;
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut policies = QueuePolicies::new();
/// 
/// // Configure business hours policies
/// let business_hours = BusinessHours {
///     start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
///     end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(), 
///     days: vec!["monday".to_string(), "tuesday".to_string(), 
///               "wednesday".to_string(), "thursday".to_string(), "friday".to_string()],
/// };
/// 
/// // Different queue priorities during business hours
/// policies.add_time_based_rule(TimeBasedRule {
///     name: "business_hours_priority".to_string(),
///     condition: business_hours,
///     priority_boost: 1.2, // 20% priority increase
///     target_queues: vec!["sales_queue".to_string(), "support_queue".to_string()],
/// })?;
/// 
/// // Peak hours handling (lunch time)
/// let peak_hours = BusinessHours {
///     start_time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
///     end_time: NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
///     days: vec!["monday".to_string(), "tuesday".to_string(), 
///               "wednesday".to_string(), "thursday".to_string(), "friday".to_string()],
/// };
/// 
/// policies.add_time_based_rule(TimeBasedRule {
///     name: "peak_hours_overflow".to_string(), 
///     condition: peak_hours,
///     priority_boost: 0.8, // Slightly lower priority to manage load
///     target_queues: vec!["overflow_queue".to_string()],
/// })?;
/// 
/// println!("Time-based policies configured");
/// # Ok(())
/// # }
/// ```
pub struct QueuePolicies {
    /// Customer tier priority mappings
    customer_priorities: HashMap<String, PriorityLevel>,
    
    /// Call type priority mappings  
    call_type_priorities: HashMap<String, PriorityLevel>,
    
    /// Queue assignment rules
    assignment_rules: Vec<AssignmentRule>,
    
    /// Service level targets by queue
    service_level_targets: HashMap<String, ServiceLevelTarget>,
    
    /// Time-based policy rules
    time_based_rules: Vec<TimeBasedRule>,
    
    /// Dynamic priority adjustment factors
    priority_factors: PriorityFactors,
    
    /// Queue capacity limits
    queue_limits: HashMap<String, QueueLimit>,
    
    /// Performance metrics cache
    performance_cache: HashMap<String, QueuePerformance>,
}

/// Priority levels for call handling
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PriorityLevel {
    /// Critical priority - immediate handling required
    Critical = 4,
    /// High priority - expedited handling
    High = 3,
    /// Normal priority - standard handling
    Normal = 2,
    /// Low priority - can be deferred
    Low = 1,
}

/// Call context information for policy decisions
#[derive(Debug, Clone)]
pub struct CallContext {
    /// Call attributes and metadata
    pub attributes: HashMap<String, String>,
    
    /// Call start time
    pub call_start_time: DateTime<Utc>,
    
    /// Current wait time in seconds
    pub wait_time_seconds: u64,
}

/// Priority calculation result
#[derive(Debug, Clone)]
pub struct PriorityResult {
    /// Calculated priority level
    pub level: PriorityLevel,
    
    /// Numeric priority score (higher = more priority)
    pub score: f64,
    
    /// Whether call should be escalated
    pub should_escalate: bool,
    
    /// Reasons for this priority assignment
    pub reasons: Vec<String>,
}

/// Queue assignment decision
#[derive(Debug, Clone)]
pub enum QueueAssignment {
    /// Assign to primary queue
    Primary(String),
    /// Assign to secondary queue with reason
    Secondary(String, String),
    /// Reject the call with reason
    Reject(String),
}

/// Service level target configuration
#[derive(Debug, Clone)]
pub struct ServiceLevelTarget {
    /// Target percentage of calls to meet service level
    pub target_percentage: f64,
    
    /// Target answer time in seconds
    pub target_time_seconds: u64,
    
    /// Time before escalation is triggered
    pub escalation_time_seconds: u64,
}

/// Service level compliance information
#[derive(Debug, Clone)]
pub struct ServiceLevelCompliance {
    /// Current service level percentage
    pub current_percentage: f64,
    
    /// Target service level percentage
    pub target_percentage: f64,
    
    /// Whether currently meeting SLA
    pub meeting_sla: bool,
    
    /// Time period for this measurement
    pub measurement_period_hours: u32,
    
    /// Recommendations for improvement
    pub recommendations: Vec<String>,
}

/// Queue assignment rule
#[derive(Debug, Clone)]
pub struct AssignmentRule {
    /// Rule name
    pub name: String,
    
    /// Conditions that must be met
    pub conditions: Vec<AssignmentCondition>,
    
    /// Target queue for assignment
    pub target_queue: String,
    
    /// Rule priority (lower = higher priority)
    pub priority: u8,
    
    /// Whether rule is currently active
    pub active: bool,
}

/// Condition for queue assignment
#[derive(Debug, Clone)]
pub enum AssignmentCondition {
    /// Customer tier matches
    CustomerTier(String),
    /// Call type matches
    CallType(String),
    /// Customer location matches
    CustomerLocation(String),
    /// Time of day within range
    TimeOfDay(chrono::NaiveTime, chrono::NaiveTime),
    /// Queue capacity below threshold
    QueueCapacity(String, u32),
    /// Custom attribute match
    CustomAttribute(String, String),
}

/// Time-based policy rule
#[derive(Debug, Clone)]
pub struct TimeBasedRule {
    /// Rule name
    pub name: String,
    
    /// Time condition for this rule
    pub condition: BusinessHours,
    
    /// Priority boost factor (1.0 = no change)
    pub priority_boost: f64,
    
    /// Queues this rule applies to
    pub target_queues: Vec<String>,
}

/// Business hours definition
#[derive(Debug, Clone)]
pub struct BusinessHours {
    /// Start time
    pub start_time: chrono::NaiveTime,
    
    /// End time
    pub end_time: chrono::NaiveTime,
    
    /// Days of week this applies to
    pub days: Vec<String>,
}

/// Priority adjustment factors
#[derive(Debug, Clone)]
pub struct PriorityFactors {
    /// Factor for wait time (higher wait = higher priority)
    pub wait_time_factor: f64,
    
    /// Factor for customer tier
    pub customer_tier_factor: f64,
    
    /// Factor for call type
    pub call_type_factor: f64,
    
    /// Factor for time of day
    pub time_of_day_factor: f64,
    
    /// Factor for queue load
    pub queue_load_factor: f64,
}

/// Queue capacity and limits
#[derive(Debug, Clone)]
pub struct QueueLimit {
    /// Maximum calls allowed in queue
    pub max_calls: u32,
    
    /// Soft limit before warnings
    pub soft_limit: u32,
    
    /// Maximum wait time before escalation
    pub max_wait_time: u64,
    
    /// Whether queue is currently accepting calls
    pub accepting_calls: bool,
}

/// Queue performance metrics
#[derive(Debug, Clone)]
pub struct QueuePerformance {
    /// Current queue size
    pub current_size: u32,
    
    /// Average wait time
    pub average_wait_time: u64,
    
    /// Service level percentage
    pub service_level: f64,
    
    /// Abandonment rate
    pub abandonment_rate: f64,
    
    /// Last updated timestamp
    pub last_updated: DateTime<Utc>,
}

impl QueuePolicies {
    /// Create a new queue policies manager
    ///
    /// Initializes a new queue policies system with default configurations.
    /// The system will be ready to handle policy decisions immediately.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::QueuePolicies;
    /// 
    /// let policies = QueuePolicies::new();
    /// println!("Queue policies initialized");
    /// ```
    pub fn new() -> Self {
        Self {
            customer_priorities: HashMap::new(),
            call_type_priorities: HashMap::new(),
            assignment_rules: Vec::new(),
            service_level_targets: HashMap::new(),
            time_based_rules: Vec::new(),
            priority_factors: PriorityFactors {
                wait_time_factor: 1.5,
                customer_tier_factor: 2.0,
                call_type_factor: 1.2,
                time_of_day_factor: 1.1,
                queue_load_factor: 1.3,
            },
            queue_limits: HashMap::new(),
            performance_cache: HashMap::new(),
        }
    }
    
    /// Set priority level for a customer tier
    ///
    /// Configures the priority level for calls from customers of a specific tier.
    ///
    /// # Arguments
    ///
    /// * `tier` - Customer tier identifier (e.g., "vip", "gold", "silver")
    /// * `priority` - Priority level to assign
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::{QueuePolicies, PriorityLevel};
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut policies = QueuePolicies::new();
    /// policies.set_customer_priority("vip", PriorityLevel::Critical)?;
    /// policies.set_customer_priority("premium", PriorityLevel::High)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_customer_priority(&mut self, tier: &str, priority: PriorityLevel) -> Result<()> {
        self.customer_priorities.insert(tier.to_string(), priority);
        info!("Set customer tier '{}' priority to {:?}", tier, priority);
        Ok(())
    }
    
    /// Set priority level for a call type
    ///
    /// Configures the priority level for calls of a specific type.
    ///
    /// # Arguments
    ///
    /// * `call_type` - Type of call (e.g., "emergency", "sales", "support")
    /// * `priority` - Priority level to assign
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::{QueuePolicies, PriorityLevel};
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut policies = QueuePolicies::new();
    /// policies.set_call_type_priority("emergency", PriorityLevel::Critical)?;
    /// policies.set_call_type_priority("sales", PriorityLevel::High)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_call_type_priority(&mut self, call_type: &str, priority: PriorityLevel) -> Result<()> {
        self.call_type_priorities.insert(call_type.to_string(), priority);
        info!("Set call type '{}' priority to {:?}", call_type, priority);
        Ok(())
    }
    
    /// Calculate priority for a call based on context
    ///
    /// Analyzes call context and returns calculated priority with reasoning.
    ///
    /// # Arguments
    ///
    /// * `context` - Call context containing attributes and metadata
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::{QueuePolicies, CallContext};
    /// use std::collections::HashMap;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let policies = QueuePolicies::new();
    /// 
    /// let mut attrs = HashMap::new();
    /// attrs.insert("customer_tier".to_string(), "vip".to_string());
    /// attrs.insert("call_type".to_string(), "support".to_string());
    /// 
    /// let context = CallContext::new(attrs);
    /// let priority = policies.calculate_priority(&context)?;
    /// 
    /// println!("Priority: {:?} (score: {})", priority.level, priority.score);
    /// # Ok(())
    /// # }
    /// ```
    pub fn calculate_priority(&self, context: &CallContext) -> Result<PriorityResult> {
        let mut score = 0.0;
        let mut reasons = Vec::new();
        let mut base_priority = PriorityLevel::Normal;
        
        // Check customer tier priority
        if let Some(tier) = context.attributes.get("customer_tier") {
            if let Some(&priority) = self.customer_priorities.get(tier) {
                base_priority = std::cmp::max(base_priority, priority);
                score += (priority as u8 as f64) * self.priority_factors.customer_tier_factor;
                reasons.push(format!("Customer tier '{}' -> {:?}", tier, priority));
            }
        }
        
        // Check call type priority
        if let Some(call_type) = context.attributes.get("call_type") {
            if let Some(&priority) = self.call_type_priorities.get(call_type) {
                base_priority = std::cmp::max(base_priority, priority);
                score += (priority as u8 as f64) * self.priority_factors.call_type_factor;
                reasons.push(format!("Call type '{}' -> {:?}", call_type, priority));
            }
        }
        
        // Apply wait time factor
        let wait_time_boost = (context.wait_time_seconds as f64 / 60.0) * self.priority_factors.wait_time_factor;
        score += wait_time_boost;
        if wait_time_boost > 0.5 {
            reasons.push(format!("Wait time {}s adds {:.1} priority", context.wait_time_seconds, wait_time_boost));
        }
        
        // Apply time-based rules
        for rule in &self.time_based_rules {
            if self.matches_business_hours(&rule.condition) {
                score *= rule.priority_boost;
                reasons.push(format!("Time-based rule '{}' applied", rule.name));
            }
        }
        
        // Determine if escalation is needed
        let should_escalate = context.wait_time_seconds > 180 || // 3 minutes
                             base_priority == PriorityLevel::Critical;
        
        if should_escalate {
            reasons.push("Call meets escalation criteria".to_string());
        }
        
        Ok(PriorityResult {
            level: base_priority,
            score,
            should_escalate,
            reasons,
        })
    }
    
    /// Assign a call to an appropriate queue
    ///
    /// Determines the best queue assignment based on call context and policies.
    ///
    /// # Arguments
    ///
    /// * `context` - Call context for assignment decision
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::{QueuePolicies, CallContext};
    /// use std::collections::HashMap;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let policies = QueuePolicies::new();
    /// 
    /// let mut attrs = HashMap::new();
    /// attrs.insert("customer_tier".to_string(), "vip".to_string());
    /// 
    /// let context = CallContext::new(attrs);
    /// let assignment = policies.assign_queue(&context)?;
    /// 
    /// println!("Queue assignment: {:?}", assignment);
    /// # Ok(())
    /// # }
    /// ```
    pub fn assign_queue(&self, context: &CallContext) -> Result<QueueAssignment> {
        // Apply assignment rules in priority order
        for rule in &self.assignment_rules {
            if !rule.active {
                continue;
            }
            
            if self.matches_assignment_conditions(&rule.conditions, context) {
                return Ok(QueueAssignment::Primary(rule.target_queue.clone()));
            }
        }
        
        // Default assignment logic
        if let Some(tier) = context.attributes.get("customer_tier") {
            match tier.as_str() {
                "vip" | "platinum" => Ok(QueueAssignment::Primary("vip_queue".to_string())),
                "gold" | "premium" => Ok(QueueAssignment::Primary("premium_queue".to_string())), 
                _ => Ok(QueueAssignment::Primary("general_queue".to_string())),
            }
        } else {
            Ok(QueueAssignment::Primary("general_queue".to_string()))
        }
    }
    
    /// Set service level target for a queue
    ///
    /// Configures SLA targets and escalation rules for a specific queue.
    ///
    /// # Arguments
    ///
    /// * `queue_id` - Queue identifier
    /// * `target` - Service level target configuration
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::{QueuePolicies, ServiceLevelTarget};
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut policies = QueuePolicies::new();
    /// 
    /// let target = ServiceLevelTarget {
    ///     target_percentage: 90.0,
    ///     target_time_seconds: 20,
    ///     escalation_time_seconds: 90,
    /// };
    /// 
    /// policies.set_service_level_target("vip_queue", target)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_service_level_target(&mut self, queue_id: &str, target: ServiceLevelTarget) -> Result<()> {
        self.service_level_targets.insert(queue_id.to_string(), target.clone());
        info!("Set service level target for queue '{}': {:.1}% in {}s", 
              queue_id, target.target_percentage, target.target_time_seconds);
        Ok(())
    }
    
    /// Check service level compliance for a queue
    ///
    /// Analyzes current performance against SLA targets.
    ///
    /// # Arguments
    ///
    /// * `queue_id` - Queue to check compliance for
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::QueuePolicies;
    /// 
    /// # async fn example(policies: QueuePolicies) -> Result<(), Box<dyn std::error::Error>> {
    /// let compliance = policies.check_service_level_compliance("vip_queue").await?;
    /// 
    /// if compliance.meeting_sla {
    ///     println!("✅ Queue meeting SLA: {:.1}%", compliance.current_percentage);
    /// } else {
    ///     println!("❌ Queue below SLA: {:.1}% (target: {:.1}%)", 
    ///              compliance.current_percentage, compliance.target_percentage);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn check_service_level_compliance(&self, queue_id: &str) -> Result<ServiceLevelCompliance> {
        let target = self.service_level_targets.get(queue_id)
            .ok_or_else(|| CallCenterError::NotFound(format!("No SLA target for queue {}", queue_id)))?;
        
        // TODO: Get actual performance metrics from queue system
        let current_percentage = 85.0; // Placeholder
        let meeting_sla = current_percentage >= target.target_percentage;
        
        let mut recommendations = Vec::new();
        if !meeting_sla {
            recommendations.push("Consider adding more agents to this queue".to_string());
            recommendations.push("Review call routing efficiency".to_string());
        }
        
        Ok(ServiceLevelCompliance {
            current_percentage,
            target_percentage: target.target_percentage,
            meeting_sla,
            measurement_period_hours: 24,
            recommendations,
        })
    }
    
    /// Add a time-based policy rule
    ///
    /// Adds a rule that modifies queue behavior based on time conditions.
    ///
    /// # Arguments
    ///
    /// * `rule` - Time-based rule configuration
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::{QueuePolicies, TimeBasedRule, BusinessHours};
    /// use chrono::NaiveTime;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut policies = QueuePolicies::new();
    /// 
    /// let rule = TimeBasedRule {
    ///     name: "peak_hours".to_string(),
    ///     condition: BusinessHours {
    ///         start_time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
    ///         end_time: NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
    ///         days: vec!["monday".to_string(), "friday".to_string()],
    ///     },
    ///     priority_boost: 1.3,
    ///     target_queues: vec!["sales_queue".to_string()],
    /// };
    /// 
    /// policies.add_time_based_rule(rule)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_time_based_rule(&mut self, rule: TimeBasedRule) -> Result<()> {
        info!("Adding time-based rule: {}", rule.name);
        self.time_based_rules.push(rule);
        Ok(())
    }
    
    /// Add a queue assignment rule
    ///
    /// Adds a rule for automatic queue assignment based on conditions.
    ///
    /// # Arguments
    ///
    /// * `rule` - Assignment rule configuration
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::{QueuePolicies, AssignmentRule, AssignmentCondition};
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut policies = QueuePolicies::new();
    /// 
    /// let rule = AssignmentRule {
    ///     name: "vip_routing".to_string(),
    ///     conditions: vec![AssignmentCondition::CustomerTier("vip".to_string())],
    ///     target_queue: "vip_queue".to_string(),
    ///     priority: 1,
    ///     active: true,
    /// };
    /// 
    /// policies.add_assignment_rule(rule)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_assignment_rule(&mut self, rule: AssignmentRule) -> Result<()> {
        info!("Adding assignment rule: {}", rule.name);
        self.assignment_rules.push(rule);
        // Sort by priority
        self.assignment_rules.sort_by_key(|r| r.priority);
        Ok(())
    }
    
    /// Set queue capacity limits
    ///
    /// Configures capacity limits and thresholds for a queue.
    ///
    /// # Arguments
    ///
    /// * `queue_id` - Queue identifier
    /// * `limit` - Queue limit configuration
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::{QueuePolicies, QueueLimit};
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut policies = QueuePolicies::new();
    /// 
    /// let limit = QueueLimit {
    ///     max_calls: 100,
    ///     soft_limit: 80,
    ///     max_wait_time: 300, // 5 minutes
    ///     accepting_calls: true,
    /// };
    /// 
    /// policies.set_queue_limit("general_queue", limit)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_queue_limit(&mut self, queue_id: &str, limit: QueueLimit) -> Result<()> {
        self.queue_limits.insert(queue_id.to_string(), limit.clone());
        info!("Set queue limit for '{}': max {} calls", queue_id, limit.max_calls);
        Ok(())
    }
    
    // Private helper methods
    
    fn matches_business_hours(&self, hours: &BusinessHours) -> bool {
        let now = Utc::now();
        let current_time = now.time();
        let current_day = format!("{:?}", now.weekday()).to_lowercase();
        
        hours.days.contains(&current_day) &&
        current_time >= hours.start_time &&
        current_time <= hours.end_time
    }
    
    fn matches_assignment_conditions(&self, conditions: &[AssignmentCondition], context: &CallContext) -> bool {
        conditions.iter().all(|condition| {
            match condition {
                AssignmentCondition::CustomerTier(tier) => {
                    context.attributes.get("customer_tier").map_or(false, |t| t == tier)
                }
                AssignmentCondition::CallType(call_type) => {
                    context.attributes.get("call_type").map_or(false, |t| t == call_type)
                }
                AssignmentCondition::CustomerLocation(location) => {
                    context.attributes.get("customer_location").map_or(false, |l| l == location)
                }
                AssignmentCondition::CustomAttribute(key, value) => {
                    context.attributes.get(key).map_or(false, |v| v == value)
                }
                _ => {
                    // TODO: Implement other condition types
                    warn!("🚧 Assignment condition not fully implemented: {:?}", condition);
                    false
                }
            }
        })
    }
}

impl CallContext {
    /// Create a new call context
    ///
    /// # Arguments
    ///
    /// * `attributes` - Call attributes and metadata
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::CallContext;
    /// use std::collections::HashMap;
    /// 
    /// let mut attrs = HashMap::new();
    /// attrs.insert("customer_id".to_string(), "12345".to_string());
    /// 
    /// let context = CallContext::new(attrs);
    /// ```
    pub fn new(attributes: HashMap<String, String>) -> Self {
        Self {
            attributes,
            call_start_time: Utc::now(),
            wait_time_seconds: 0,
        }
    }
    
    /// Update wait time for this call
    ///
    /// # Arguments
    ///
    /// * `seconds` - Current wait time in seconds
    pub fn update_wait_time(&mut self, seconds: u64) {
        self.wait_time_seconds = seconds;
    }
}

impl Default for QueuePolicies {
    fn default() -> Self {
        Self::new()
    }
} 