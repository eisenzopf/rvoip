//! # Routing Policies Implementation
//!
//! This module provides comprehensive routing policies and business rules for
//! intelligent call distribution in the call center. It enables sophisticated
//! routing decisions based on time, agent skills, customer priority, and
//! business requirements.

use std::collections::HashMap;
use chrono::{DateTime, Utc, Timelike, Weekday, NaiveTime, Datelike};
use serde::{Serialize, Deserialize};
use tracing::{info, debug, warn};

use crate::error::{CallCenterError, Result};
use crate::agent::{AgentId, AgentStatus};

/// # Routing Policies for Business Rules and Call Distribution
///
/// The `RoutingPolicies` system provides comprehensive business rules and policies
/// for intelligent call routing decisions. It enables call centers to implement
/// sophisticated routing strategies based on time, agent capabilities, customer
/// priority, and business requirements.
///
/// ## Key Features
///
/// - **Time-Based Routing**: Different routing rules for business hours, after hours, holidays
/// - **Skill-Based Rules**: Advanced skill matching with proficiency levels and requirements
/// - **Priority Routing**: VIP customers, escalations, and priority-based queue assignment
/// - **Geographic Routing**: Location-based routing preferences and restrictions
/// - **Load Balancing**: Intelligent distribution across available agents
/// - **Business Rules**: Configurable rules for complex routing scenarios
///
/// ## Policy Types
///
/// ### Time-Based Policies
/// - Business hours vs. after hours routing
/// - Holiday and weekend special handling
/// - Time zone aware routing
/// - Scheduled maintenance windows
///
/// ### Customer-Based Policies
/// - VIP customer priority routing
/// - Customer tier-based assignment
/// - Account-specific routing rules
/// - Geographic customer preferences
///
/// ### Agent-Based Policies
/// - Skill level requirements
/// - Department-based routing
/// - Performance-based assignment
/// - Workload balancing rules
///
/// ### Queue-Based Policies
/// - Queue priority levels
/// - Overflow and escalation rules
/// - SLA-based routing decisions
/// - Capacity management policies
///
/// ## Examples
///
/// ### Basic Time-Based Routing
///
/// ```rust
/// use rvoip_call_engine::routing::{RoutingPolicies, TimeBasedRule};
/// use chrono::NaiveTime;
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut policies = RoutingPolicies::new();
/// 
/// // Configure business hours
/// policies.set_business_hours(
///     NaiveTime::from_hms_opt(9, 0, 0).unwrap(),  // 9:00 AM
///     NaiveTime::from_hms_opt(17, 0, 0).unwrap(), // 5:00 PM
///     vec!["monday", "tuesday", "wednesday", "thursday", "friday"]
/// )?;
/// 
/// // Configure after-hours routing
/// policies.set_after_hours_queue("after_hours_support")?;
/// 
/// // Check if current time is business hours
/// if policies.is_business_hours() {
///     println!("Routing to regular agents");
/// } else {
///     println!("Routing to after-hours queue");
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### VIP Customer Routing
///
/// ```rust
/// use rvoip_call_engine::routing::policies::{RoutingPolicies, SkillRequirement, SkillLevel};
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut policies = RoutingPolicies::new();
/// 
/// // Configure VIP customer routing
/// policies.add_vip_customer("customer-12345")?;
/// policies.set_vip_queue("vip_support")?;
/// policies.set_vip_escalation_time(30)?; // 30 seconds max wait
/// 
/// // Check routing for a customer
/// let customer_id = "customer-12345";
/// if policies.is_vip_customer(customer_id) {
///     let queue = policies.get_vip_queue();
///     println!("VIP customer {} routed to {}", customer_id, queue.unwrap());
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Skill-Based Routing Rules
///
/// ```rust
/// use rvoip_call_engine::routing::policies::{RoutingPolicies, SkillRequirement, SkillLevel};
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut policies = RoutingPolicies::new();
/// 
/// // Configure skill requirements for different call types
/// policies.add_skill_requirement(
///     "technical_support",
///     vec![
///         SkillRequirement::new("technical", SkillLevel::Intermediate, true), // Required
///         SkillRequirement::new("english", SkillLevel::Advanced, true),      // Required
///         SkillRequirement::new("tier2", SkillLevel::Basic, false),          // Preferred
///     ]
/// )?;
/// 
/// // Configure escalation rules
/// policies.add_escalation_rule(
///     "technical_support",
///     120, // After 2 minutes, escalate to tier 2
///     vec![SkillRequirement::new("tier2", SkillLevel::Advanced, true)]
/// )?;
/// 
/// println!("Skill-based routing configured");
/// # Ok(())
/// # }
/// ```
///
/// ### Geographic Routing
///
/// ```rust
/// use rvoip_call_engine::routing::policies::RoutingPolicies;
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut policies = RoutingPolicies::new();
/// 
/// // Configure geographic routing preferences
/// policies.add_geographic_rule(
///     "US",
///     vec!["agent-us-001", "agent-us-002"],
///     "english"
/// )?;
/// 
/// policies.add_geographic_rule(
///     "CA",
///     vec!["agent-ca-001", "agent-ca-002"],
///     "english,french"
/// )?;
/// 
/// // Route based on caller location
/// let caller_country = "US";
/// if let Some(preferred_agents) = policies.get_geographic_agents(caller_country) {
///     println!("Preferred agents for {}: {:?}", caller_country, preferred_agents);
/// }
/// # Ok(())
/// # }
/// ```
pub struct RoutingPolicies {
    /// Time-based routing rules
    time_rules: TimeBasedRules,
    
    /// Customer priority and VIP rules
    customer_rules: CustomerRules,
    
    /// Skill-based routing requirements
    skill_rules: SkillBasedRules,
    
    /// Geographic routing preferences
    geographic_rules: GeographicRules,
    
    /// Load balancing configuration
    load_balancing: LoadBalancingRules,
    
    /// Emergency and failover rules
    emergency_rules: EmergencyRules,
    
    /// Custom business rules
    custom_rules: HashMap<String, CustomRule>,
}

/// Time-based routing rules and schedules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeBasedRules {
    /// Business hours definition
    pub business_hours: BusinessHours,
    
    /// After-hours routing configuration
    pub after_hours_queue: Option<String>,
    
    /// Holiday routing overrides
    pub holiday_rules: HashMap<String, HolidayRule>,
    
    /// Time zone for routing decisions
    pub timezone: String,
    
    /// Maintenance windows
    pub maintenance_windows: Vec<MaintenanceWindow>,
}

/// Business hours configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusinessHours {
    /// Start time for business hours
    pub start_time: NaiveTime,
    
    /// End time for business hours
    pub end_time: NaiveTime,
    
    /// Days of week for business hours
    pub business_days: Vec<String>,
    
    /// Time zone identifier
    pub timezone: String,
}

/// Holiday routing rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolidayRule {
    /// Holiday name
    pub name: String,
    
    /// Holiday date
    pub date: DateTime<Utc>,
    
    /// Special routing queue for this holiday
    pub queue: Option<String>,
    
    /// Whether to use after-hours routing
    pub use_after_hours: bool,
}

/// Maintenance window definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintenanceWindow {
    /// Window start time
    pub start: DateTime<Utc>,
    
    /// Window end time
    pub end: DateTime<Utc>,
    
    /// Maintenance routing queue
    pub maintenance_queue: String,
    
    /// Message for callers during maintenance
    pub message: String,
}

/// Customer priority and VIP rules
#[derive(Debug, Clone)]
pub struct CustomerRules {
    /// VIP customer identifiers
    pub vip_customers: HashMap<String, VipCustomerConfig>,
    
    /// Customer tier definitions
    pub customer_tiers: HashMap<String, CustomerTier>,
    
    /// VIP routing queue
    pub vip_queue: Option<String>,
    
    /// VIP escalation time (seconds)
    pub vip_escalation_time: u64,
    
    /// Account-specific routing rules
    pub account_rules: HashMap<String, AccountRule>,
}

/// VIP customer configuration
#[derive(Debug, Clone)]
pub struct VipCustomerConfig {
    /// Customer identifier
    pub customer_id: String,
    
    /// VIP tier level
    pub tier: u8,
    
    /// Dedicated agent if any
    pub dedicated_agent: Option<String>,
    
    /// Maximum wait time before escalation
    pub max_wait_time: u64,
}

/// Customer tier definition
#[derive(Debug, Clone)]
pub struct CustomerTier {
    /// Tier name
    pub name: String,
    
    /// Priority level (lower = higher priority)
    pub priority: u8,
    
    /// Preferred queue
    pub preferred_queue: Option<String>,
    
    /// Maximum wait time
    pub max_wait_time: u64,
}

/// Account-specific routing rule
#[derive(Debug, Clone)]
pub struct AccountRule {
    /// Account identifier
    pub account_id: String,
    
    /// Dedicated agents for this account
    pub dedicated_agents: Vec<String>,
    
    /// Preferred skills for this account
    pub preferred_skills: Vec<String>,
    
    /// Account-specific queue
    pub account_queue: Option<String>,
}

/// Skill-based routing requirements and rules
#[derive(Debug, Clone)]
pub struct SkillBasedRules {
    /// Skill requirements by call type
    pub call_type_requirements: HashMap<String, Vec<SkillRequirement>>,
    
    /// Escalation rules based on skills
    pub escalation_rules: HashMap<String, EscalationRule>,
    
    /// Skill proficiency levels
    pub skill_levels: HashMap<String, SkillDefinition>,
    
    /// Fallback rules when skills not available
    pub fallback_rules: FallbackRules,
}

/// Individual skill requirement
#[derive(Debug, Clone)]
pub struct SkillRequirement {
    /// Skill name
    pub skill_name: String,
    
    /// Required proficiency level
    pub level: SkillLevel,
    
    /// Whether this skill is required or preferred
    pub required: bool,
    
    /// Weight for skill matching algorithm
    pub weight: f32,
}

/// Skill proficiency levels
#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub enum SkillLevel {
    /// Basic proficiency
    Basic,
    /// Intermediate proficiency
    Intermediate,
    /// Advanced proficiency
    Advanced,
    /// Expert level proficiency
    Expert,
}

/// Skill definition and configuration
#[derive(Debug, Clone)]
pub struct SkillDefinition {
    /// Skill name
    pub name: String,
    
    /// Skill category
    pub category: String,
    
    /// Skill description
    pub description: String,
    
    /// Whether this skill can be substituted
    pub substitutable: bool,
    
    /// Alternative skills that can substitute
    pub substitutes: Vec<String>,
}

/// Escalation rule based on skills and time
#[derive(Debug, Clone)]
pub struct EscalationRule {
    /// Call type this rule applies to
    pub call_type: String,
    
    /// Time in seconds before escalation
    pub escalation_time: u64,
    
    /// Required skills after escalation
    pub escalated_skills: Vec<SkillRequirement>,
    
    /// Target queue for escalation
    pub escalated_queue: Option<String>,
}

/// Fallback rules when primary routing fails
#[derive(Debug, Clone)]
pub struct FallbackRules {
    /// Enable fallback to agents without perfect skill match
    pub allow_partial_match: bool,
    
    /// Minimum skill match percentage required
    pub min_match_percentage: f32,
    
    /// Fallback queue when no agents available
    pub fallback_queue: String,
    
    /// Time to wait before using fallback rules
    pub fallback_delay: u64,
}

/// Geographic routing rules and preferences
#[derive(Debug, Clone)]
pub struct GeographicRules {
    /// Country-based agent preferences
    pub country_preferences: HashMap<String, CountryRule>,
    
    /// Language preferences by region
    pub language_preferences: HashMap<String, Vec<String>>,
    
    /// Time zone aware routing
    pub timezone_aware: bool,
    
    /// Default routing for unknown locations
    pub default_routing: Option<String>,
}

/// Country-specific routing rule
#[derive(Debug, Clone)]
pub struct CountryRule {
    /// Country code
    pub country_code: String,
    
    /// Preferred agents for this country
    pub preferred_agents: Vec<String>,
    
    /// Required languages
    pub required_languages: Vec<String>,
    
    /// Preferred time zone
    pub timezone: Option<String>,
}

/// Load balancing rules and algorithms
#[derive(Debug, Clone)]
pub struct LoadBalancingRules {
    /// Load balancing strategy
    pub strategy: LoadBalancingStrategy,
    
    /// Maximum calls per agent
    pub max_calls_per_agent: u32,
    
    /// Weight factors for load balancing
    pub weight_factors: WeightFactors,
    
    /// Enable adaptive load balancing
    pub adaptive_balancing: bool,
}

/// Load balancing strategies
#[derive(Debug, Clone)]
pub enum LoadBalancingStrategy {
    /// Round robin distribution
    RoundRobin,
    /// Route to least busy agent
    LeastBusy,
    /// Weighted round robin
    WeightedRoundRobin,
    /// Performance-based routing
    PerformanceBased,
    /// Random distribution
    Random,
}

/// Weight factors for load balancing decisions
#[derive(Debug, Clone)]
pub struct WeightFactors {
    /// Weight for agent performance
    pub performance_weight: f32,
    
    /// Weight for current workload
    pub workload_weight: f32,
    
    /// Weight for skill match quality
    pub skill_match_weight: f32,
    
    /// Weight for customer satisfaction scores
    pub satisfaction_weight: f32,
}

/// Emergency and failover routing rules
#[derive(Debug, Clone)]
pub struct EmergencyRules {
    /// Emergency routing queue
    pub emergency_queue: String,
    
    /// System overload threshold
    pub overload_threshold: f32,
    
    /// Failover routing rules
    pub failover_rules: Vec<FailoverRule>,
    
    /// Emergency contact information
    pub emergency_contacts: Vec<String>,
}

/// Failover rule definition
#[derive(Debug, Clone)]
pub struct FailoverRule {
    /// Trigger condition
    pub trigger: FailoverTrigger,
    
    /// Action to take
    pub action: FailoverAction,
    
    /// Priority of this rule
    pub priority: u8,
}

/// Failover trigger conditions
#[derive(Debug, Clone)]
pub enum FailoverTrigger {
    /// System capacity exceeded
    CapacityExceeded(f32),
    /// All agents unavailable
    NoAgentsAvailable,
    /// Queue wait time exceeded
    WaitTimeExceeded(u64),
    /// Service level below threshold
    ServiceLevelBelow(f32),
}

/// Failover actions
#[derive(Debug, Clone)]
pub enum FailoverAction {
    /// Route to specific queue
    RouteToQueue(String),
    /// Play message and hangup
    PlayMessage(String),
    /// Forward to external number
    ForwardTo(String),
    /// Enable overflow to partner center
    EnableOverflow,
}

/// Custom business rule
#[derive(Debug, Clone)]
pub struct CustomRule {
    /// Rule name
    pub name: String,
    
    /// Rule description
    pub description: String,
    
    /// Rule conditions (simplified as key-value pairs)
    pub conditions: HashMap<String, String>,
    
    /// Rule actions
    pub actions: HashMap<String, String>,
    
    /// Rule priority
    pub priority: u8,
}

impl RoutingPolicies {
    /// Create a new routing policies instance
    ///
    /// Initializes routing policies with default settings. Can be customized
    /// using the various configuration methods.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::policies::RoutingPolicies;
    /// 
    /// let policies = RoutingPolicies::new();
    /// println!("Routing policies initialized with defaults");
    /// ```
    pub fn new() -> Self {
        Self {
            time_rules: TimeBasedRules {
                business_hours: BusinessHours {
                    start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
                    business_days: vec![
                        "monday".to_string(),
                        "tuesday".to_string(),
                        "wednesday".to_string(),
                        "thursday".to_string(),
                        "friday".to_string(),
                    ],
                    timezone: "UTC".to_string(),
                },
                after_hours_queue: None,
                holiday_rules: HashMap::new(),
                timezone: "UTC".to_string(),
                maintenance_windows: Vec::new(),
            },
            customer_rules: CustomerRules {
                vip_customers: HashMap::new(),
                customer_tiers: HashMap::new(),
                vip_queue: None,
                vip_escalation_time: 30,
                account_rules: HashMap::new(),
            },
            skill_rules: SkillBasedRules {
                call_type_requirements: HashMap::new(),
                escalation_rules: HashMap::new(),
                skill_levels: HashMap::new(),
                fallback_rules: FallbackRules {
                    allow_partial_match: true,
                    min_match_percentage: 0.6,
                    fallback_queue: "general".to_string(),
                    fallback_delay: 120,
                },
            },
            geographic_rules: GeographicRules {
                country_preferences: HashMap::new(),
                language_preferences: HashMap::new(),
                timezone_aware: false,
                default_routing: Some("general".to_string()),
            },
            load_balancing: LoadBalancingRules {
                strategy: LoadBalancingStrategy::LeastBusy,
                max_calls_per_agent: 3,
                weight_factors: WeightFactors {
                    performance_weight: 0.3,
                    workload_weight: 0.4,
                    skill_match_weight: 0.2,
                    satisfaction_weight: 0.1,
                },
                adaptive_balancing: true,
            },
            emergency_rules: EmergencyRules {
                emergency_queue: "emergency".to_string(),
                overload_threshold: 0.9,
                failover_rules: Vec::new(),
                emergency_contacts: Vec::new(),
            },
            custom_rules: HashMap::new(),
        }
    }
    
    /// Configure business hours
    ///
    /// Sets the business hours for the call center, affecting time-based routing decisions.
    ///
    /// # Arguments
    ///
    /// * `start_time` - Start of business hours
    /// * `end_time` - End of business hours
    /// * `business_days` - Days of the week for business hours
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::RoutingPolicies;
    /// use chrono::NaiveTime;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut policies = RoutingPolicies::new();
    /// 
    /// policies.set_business_hours(
    ///     NaiveTime::from_hms_opt(8, 30, 0).unwrap(),  // 8:30 AM
    ///     NaiveTime::from_hms_opt(18, 0, 0).unwrap(),  // 6:00 PM
    ///     vec!["monday", "tuesday", "wednesday", "thursday", "friday", "saturday"]
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_business_hours(
        &mut self,
        start_time: NaiveTime,
        end_time: NaiveTime,
        business_days: Vec<&str>,
    ) -> Result<()> {
        self.time_rules.business_hours.start_time = start_time;
        self.time_rules.business_hours.end_time = end_time;
        self.time_rules.business_hours.business_days = business_days
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        
        info!("📅 Business hours configured: {} to {} on {:?}",
              start_time.format("%H:%M"),
              end_time.format("%H:%M"),
              self.time_rules.business_hours.business_days);
        
        Ok(())
    }
    
    /// Set after-hours routing queue
    ///
    /// Configures the queue to use for calls received outside business hours.
    ///
    /// # Arguments
    ///
    /// * `queue_name` - Name of the after-hours queue
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::RoutingPolicies;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut policies = RoutingPolicies::new();
    /// policies.set_after_hours_queue("night_support")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_after_hours_queue(&mut self, queue_name: &str) -> Result<()> {
        self.time_rules.after_hours_queue = Some(queue_name.to_string());
        info!("🌙 After-hours queue set to: {}", queue_name);
        Ok(())
    }
    
    /// Check if current time is within business hours
    ///
    /// Returns true if the current time falls within the configured business hours.
    ///
    /// # Returns
    ///
    /// `true` if currently in business hours, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::RoutingPolicies;
    /// 
    /// # fn example() {
    /// let policies = RoutingPolicies::new();
    /// 
    /// if policies.is_business_hours() {
    ///     println!("Currently in business hours");
    /// } else {
    ///     println!("Currently outside business hours");
    /// }
    /// # }
    /// ```
    pub fn is_business_hours(&self) -> bool {
        let now = Utc::now();
        let current_time = now.time();
        
        // TODO: Implement proper timezone handling
        let current_weekday = match now.weekday() {
            Weekday::Mon => "monday",
            Weekday::Tue => "tuesday", 
            Weekday::Wed => "wednesday",
            Weekday::Thu => "thursday",
            Weekday::Fri => "friday",
            Weekday::Sat => "saturday",
            Weekday::Sun => "sunday",
        };
        
        let is_business_day = self.time_rules.business_hours.business_days
            .contains(&current_weekday.to_string());
        
        let in_time_range = current_time >= self.time_rules.business_hours.start_time
            && current_time <= self.time_rules.business_hours.end_time;
        
        is_business_day && in_time_range
    }
    
    /// Add a VIP customer
    ///
    /// Registers a customer as VIP for priority routing treatment.
    ///
    /// # Arguments
    ///
    /// * `customer_id` - Unique identifier for the VIP customer
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::RoutingPolicies;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut policies = RoutingPolicies::new();
    /// policies.add_vip_customer("enterprise-customer-001")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_vip_customer(&mut self, customer_id: &str) -> Result<()> {
        let vip_config = VipCustomerConfig {
            customer_id: customer_id.to_string(),
            tier: 1,
            dedicated_agent: None,
            max_wait_time: 30,
        };
        
        self.customer_rules.vip_customers.insert(customer_id.to_string(), vip_config);
        info!("⭐ VIP customer added: {}", customer_id);
        Ok(())
    }
    
    /// Check if a customer is VIP
    ///
    /// Returns true if the specified customer is registered as VIP.
    ///
    /// # Arguments
    ///
    /// * `customer_id` - Customer identifier to check
    ///
    /// # Returns
    ///
    /// `true` if customer is VIP, `false` otherwise.
    pub fn is_vip_customer(&self, customer_id: &str) -> bool {
        self.customer_rules.vip_customers.contains_key(customer_id)
    }
    
    /// Set VIP routing queue
    ///
    /// Configures the queue to use for VIP customers.
    ///
    /// # Arguments
    ///
    /// * `queue_name` - Name of the VIP queue
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::RoutingPolicies;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut policies = RoutingPolicies::new();
    /// policies.set_vip_queue("platinum_support")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_vip_queue(&mut self, queue_name: &str) -> Result<()> {
        self.customer_rules.vip_queue = Some(queue_name.to_string());
        info!("⭐ VIP queue set to: {}", queue_name);
        Ok(())
    }
    
    /// Get VIP queue name
    ///
    /// Returns the configured VIP queue name if set.
    pub fn get_vip_queue(&self) -> Option<&String> {
        self.customer_rules.vip_queue.as_ref()
    }
    
    /// Set VIP escalation time
    ///
    /// Sets the maximum wait time for VIP customers before escalation.
    ///
    /// # Arguments
    ///
    /// * `seconds` - Maximum wait time in seconds
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::RoutingPolicies;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut policies = RoutingPolicies::new();
    /// policies.set_vip_escalation_time(15)?; // 15 seconds max wait
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_vip_escalation_time(&mut self, seconds: u64) -> Result<()> {
        self.customer_rules.vip_escalation_time = seconds;
        info!("⭐ VIP escalation time set to: {}s", seconds);
        Ok(())
    }
    
    /// Add skill requirement for a call type
    ///
    /// Configures the required and preferred skills for a specific type of call.
    ///
    /// # Arguments
    ///
    /// * `call_type` - Type of call (e.g., "technical_support", "billing")
    /// * `requirements` - List of skill requirements
    ///
    /// # Examples
/// 
/// ```rust
/// use rvoip_call_engine::routing::policies::{RoutingPolicies, SkillRequirement, SkillLevel};
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut policies = RoutingPolicies::new();
/// 
/// policies.add_skill_requirement(
///     "billing_support",
///     vec![
///         SkillRequirement::new("billing", SkillLevel::Advanced, true),
///         SkillRequirement::new("english", SkillLevel::Intermediate, true),
///         SkillRequirement::new("accounting", SkillLevel::Basic, false),
///     ]
/// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_skill_requirement(
        &mut self,
        call_type: &str,
        requirements: Vec<SkillRequirement>,
    ) -> Result<()> {
        self.skill_rules.call_type_requirements.insert(call_type.to_string(), requirements.clone());
        info!("🎯 Skill requirements added for call type '{}': {} skills",
              call_type, requirements.len());
        Ok(())
    }
    
    /// Add escalation rule
    ///
    /// Configures automatic escalation rules for calls that wait too long.
    ///
    /// # Arguments
    ///
    /// * `call_type` - Type of call this rule applies to
    /// * `escalation_time` - Time in seconds before escalation
    /// * `escalated_skills` - Skills required after escalation
    ///
    /// # Examples
/// 
/// ```rust
/// use rvoip_call_engine::routing::policies::{RoutingPolicies, SkillRequirement, SkillLevel};
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut policies = RoutingPolicies::new();
/// 
/// policies.add_escalation_rule(
///     "technical_support",
///     180, // 3 minutes
///     vec![SkillRequirement::new("senior_tech", SkillLevel::Expert, true)]
/// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_escalation_rule(
        &mut self,
        call_type: &str,
        escalation_time: u64,
        escalated_skills: Vec<SkillRequirement>,
    ) -> Result<()> {
        let rule = EscalationRule {
            call_type: call_type.to_string(),
            escalation_time,
            escalated_skills,
            escalated_queue: None,
        };
        
        self.skill_rules.escalation_rules.insert(call_type.to_string(), rule);
        info!("⬆️ Escalation rule added for '{}': {}s timeout", call_type, escalation_time);
        Ok(())
    }
    
    /// Add geographic routing rule
    ///
    /// Configures preferred agents and languages for specific geographic regions.
    ///
    /// # Arguments
    ///
    /// * `country_code` - ISO country code
    /// * `preferred_agents` - List of preferred agent IDs
    /// * `languages` - Comma-separated list of required languages
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::RoutingPolicies;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut policies = RoutingPolicies::new();
    /// 
    /// policies.add_geographic_rule(
    ///     "FR",
    ///     vec!["agent-paris-001", "agent-lyon-002"],
    ///     "french,english"
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_geographic_rule(
        &mut self,
        country_code: &str,
        preferred_agents: Vec<&str>,
        languages: &str,
    ) -> Result<()> {
        let agents_count = preferred_agents.len();
        let rule = CountryRule {
            country_code: country_code.to_string(),
            preferred_agents: preferred_agents.into_iter().map(|s| s.to_string()).collect(),
            required_languages: languages.split(',').map(|s| s.trim().to_string()).collect(),
            timezone: None,
        };
        
        self.geographic_rules.country_preferences.insert(country_code.to_string(), rule);
        info!("🌍 Geographic rule added for {}: {} agents, languages: {}",
              country_code, agents_count, languages);
        Ok(())
    }
    
    /// Get preferred agents for a geographic region
    ///
    /// Returns the list of preferred agents for the specified country.
    ///
    /// # Arguments
    ///
    /// * `country_code` - ISO country code
    ///
    /// # Returns
    ///
    /// `Some(Vec<String>)` with preferred agent IDs, or `None` if no rule exists.
    pub fn get_geographic_agents(&self, country_code: &str) -> Option<&Vec<String>> {
        self.geographic_rules.country_preferences
            .get(country_code)
            .map(|rule| &rule.preferred_agents)
    }
    
    /// Evaluate routing decision
    ///
    /// Applies all configured policies to determine the best routing decision
    /// for a given call context.
    ///
    /// # Arguments
    ///
    /// * `call_context` - Context information about the call
    ///
    /// # Returns
    ///
    /// Routing decision based on policies.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::{RoutingPolicies, CallContext};
    /// use std::collections::HashMap;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let policies = RoutingPolicies::new();
    /// 
    /// let mut context = HashMap::new();
    /// context.insert("customer_id".to_string(), "vip-customer-001".to_string());
    /// context.insert("call_type".to_string(), "technical_support".to_string());
    /// context.insert("country".to_string(), "US".to_string());
    /// 
    /// let decision = policies.evaluate_routing(&context);
    /// println!("Routing decision: {:?}", decision);
    /// # Ok(())
    /// # }
    /// ```
    pub fn evaluate_routing(&self, call_context: &HashMap<String, String>) -> RoutingDecision {
        // Check business hours
        if !self.is_business_hours() {
            if let Some(after_hours_queue) = &self.time_rules.after_hours_queue {
                return RoutingDecision::Queue {
                    queue_id: after_hours_queue.clone(),
                    priority: Some(5),
                    reason: Some("After hours routing".to_string()),
                };
            }
        }
        
        // Check VIP status
        if let Some(customer_id) = call_context.get("customer_id") {
            if self.is_vip_customer(customer_id) {
                if let Some(vip_queue) = &self.customer_rules.vip_queue {
                    return RoutingDecision::Queue {
                        queue_id: vip_queue.clone(),
                        priority: Some(1),
                        reason: Some("VIP customer priority routing".to_string()),
                    };
                }
            }
        }
        
        // Check geographic preferences
        if let Some(country) = call_context.get("country") {
            if let Some(agents) = self.get_geographic_agents(country) {
                if !agents.is_empty() {
                    return RoutingDecision::DirectToAgent {
                        agent_id: agents[0].clone(),
                        reason: Some("Geographic preference routing".to_string()),
                    };
                }
            }
        }
        
        // Default routing
        RoutingDecision::Queue {
            queue_id: "general".to_string(),
            priority: Some(5),
            reason: Some("Default routing".to_string()),
        }
    }
}

/// Routing decision with additional context
#[derive(Debug, Clone)]
pub enum RoutingDecision {
    /// Route directly to a specific agent
    DirectToAgent {
        /// Target agent ID
        agent_id: String,
        /// Reason for this routing decision
        reason: Option<String>,
    },
    /// Route to a queue
    Queue {
        /// Target queue ID
        queue_id: String,
        /// Queue priority (lower = higher priority)
        priority: Option<u8>,
        /// Reason for this routing decision
        reason: Option<String>,
    },
    /// Reject the call
    Reject {
        /// Reason for rejection
        reason: String,
    },
}

impl SkillRequirement {
    /// Create a new skill requirement
    ///
    /// # Arguments
    ///
    /// * `skill_name` - Name of the required skill
    /// * `level` - Required proficiency level
    /// * `required` - Whether this is required (true) or preferred (false)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::{SkillRequirement, SkillLevel};
    /// 
    /// let requirement = SkillRequirement::new("english", SkillLevel::Advanced, true);
    /// println!("Created skill requirement: {:?}", requirement);
    /// ```
    pub fn new(skill_name: &str, level: SkillLevel, required: bool) -> Self {
        Self {
            skill_name: skill_name.to_string(),
            level,
            required,
            weight: if required { 1.0 } else { 0.5 },
        }
    }
}

impl Default for RoutingPolicies {
    fn default() -> Self {
        Self::new()
    }
} 