//! # Skill-Based Agent Routing
//!
//! This module provides skill-based routing capabilities specifically for agent selection
//! in the call center context. It integrates with the agent registry to find the best
//! available agents based on skill requirements, performance metrics, and availability.

use std::collections::HashMap;
use chrono::{DateTime, Utc};
use tracing::{info, debug, warn};

use crate::error::{CallCenterError, Result};
use crate::agent::{AgentId, Agent, AgentStatus};

/// # Skill-Based Router for Agent Selection
///
/// The `SkillBasedRouter` provides intelligent agent selection based on skill requirements,
/// agent availability, and performance metrics. It serves as the primary interface for
/// routing calls to the most appropriate agents in the call center system.
///
/// ## Key Features
///
/// - **Skill Matching**: Finds agents with required skills and proficiency levels
/// - **Load Balancing**: Distributes calls across available agents efficiently
/// - **Performance Consideration**: Incorporates agent performance in selection decisions
/// - **Availability Checking**: Ensures selected agents are available for calls
/// - **Fallback Strategies**: Provides alternatives when ideal matches aren't available
/// - **Real-Time Updates**: Responds to dynamic changes in agent status and skills
///
/// ## Selection Algorithms
///
/// ### Best Match
/// - Prioritizes exact skill matches with highest proficiency
/// - Considers agent performance ratings
/// - Optimal for quality-focused scenarios
///
/// ### Load Balanced
/// - Distributes calls evenly across qualified agents
/// - Prevents agent overloading
/// - Best for high-volume scenarios
///
/// ### Performance Optimized
/// - Selects agents based on historical performance
/// - Maximizes customer satisfaction potential
/// - Ideal for critical customer interactions
///
/// ### Availability Priority
/// - Prioritizes immediately available agents
/// - Minimizes customer wait times
/// - Best for time-sensitive calls
///
/// ## Examples
///
/// ### Basic Agent Selection
///
/// ```rust
/// use rvoip_call_engine::agent::routing::{SkillBasedRouter, AgentSelectionCriteria};
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let router = SkillBasedRouter::new();
/// 
/// // Define skill requirements
/// let required_skills = vec![
///     "customer_service".to_string(),
///     "english".to_string(),
///     "billing".to_string(),
/// ];
/// 
/// // Find best available agent
/// match router.find_best_agent(&required_skills).await? {
///     Some(agent_id) => {
///         println!("✅ Selected agent: {}", agent_id);
///     }
///     None => {
///         println!("❌ No suitable agent available");
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Advanced Agent Selection with Criteria
///
/// ```rust
/// use rvoip_call_engine::agent::routing::{SkillBasedRouter, AgentSelectionCriteria, SelectionStrategy};
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let router = SkillBasedRouter::new();
/// 
/// // Define selection criteria
/// let criteria = AgentSelectionCriteria {
///     required_skills: vec!["technical_support".to_string(), "tier2".to_string()],
///     preferred_skills: vec!["windows".to_string(), "networking".to_string()],
///     min_performance_rating: Some(0.8), // Minimum 80% performance
///     max_current_calls: Some(2),         // Not handling more than 2 calls
///     strategy: SelectionStrategy::PerformanceOptimized,
///     consider_workload: true,
/// };
/// 
/// let selection = router.select_agent_with_criteria(&criteria).await?;
/// 
/// match selection {
///     Some(result) => {
///         println!("Selected agent: {}", result.agent_id);
///         println!("Match score: {:.2}", result.match_score);
///         println!("Matched skills: {:?}", result.matched_skills);
///         println!("Current workload: {}", result.current_workload);
///     }
///     None => {
///         println!("No agent meets the specified criteria");
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Load-Balanced Agent Selection
///
/// ```rust
/// use rvoip_call_engine::agent::routing::{SkillBasedRouter, LoadBalancingStrategy};
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut router = SkillBasedRouter::new();
/// 
/// // Configure load balancing
/// router.set_load_balancing_strategy(LoadBalancingStrategy::LeastBusy)?;
/// 
/// let skills = vec!["sales".to_string(), "english".to_string()];
/// 
/// // Select multiple agents for concurrent calls
/// let agents = router.select_multiple_agents(&skills, 3).await?;
/// 
/// println!("Selected {} agents for parallel calls:", agents.len());
/// for (i, selection) in agents.iter().enumerate() {
///     println!("  {}. {} (workload: {})", 
///              i + 1, selection.agent_id, selection.current_workload);
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Skill Gap Analysis and Recommendations
///
/// ```rust
/// use rvoip_call_engine::agent::SkillBasedRouter;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let router = SkillBasedRouter::new();
/// 
/// // Analyze current skill coverage
/// let coverage = router.analyze_skill_coverage().await?;
/// 
/// println!("📊 Skill Coverage Analysis:");
/// for (skill, stats) in coverage.skill_statistics {
///     println!("  {}: {} agents ({:.1}% coverage)", 
///              skill, stats.agent_count, stats.coverage_percentage);
///     
///     if stats.coverage_percentage < 50.0 {
///         println!("    ⚠️ Low coverage - consider training more agents");
///     }
/// }
/// 
/// // Get recommendations for skill improvements
/// let recommendations = router.get_skill_recommendations().await?;
/// for rec in recommendations {
///     println!("💡 {}", rec);
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Real-Time Agent Monitoring
///
/// ```rust
/// use rvoip_call_engine::agent::SkillBasedRouter;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let router = SkillBasedRouter::new();
/// 
/// // Monitor agent availability by skill
/// let availability = router.get_agent_availability_by_skill().await?;
/// 
/// println!("👥 Agent Availability by Skill:");
/// for (skill, agents) in availability {
///     let available_count = agents.iter().filter(|a| a.is_available).count();
///     println!("  {}: {}/{} agents available", 
///              skill, available_count, agents.len());
///     
///     if available_count == 0 {
///         println!("    🚨 No agents available for this skill!");
///     }
/// }
/// # Ok(())
/// # }
/// ```
pub struct SkillBasedRouter {
    /// Load balancing strategy configuration
    load_balancing_strategy: LoadBalancingStrategy,
    
    /// Performance weighting factors
    performance_weights: PerformanceWeights,
    
    /// Agent workload tracking
    agent_workloads: HashMap<String, AgentWorkload>,
    
    /// Skill demand statistics
    skill_demand: HashMap<String, SkillDemandStats>,
    
    /// Configuration for agent selection
    selection_config: SelectionConfig,
}

/// Agent selection criteria for advanced routing
#[derive(Debug, Clone)]
pub struct AgentSelectionCriteria {
    /// Skills that the agent must have
    pub required_skills: Vec<String>,
    
    /// Skills that are preferred but not required
    pub preferred_skills: Vec<String>,
    
    /// Minimum performance rating (0.0 - 1.0)
    pub min_performance_rating: Option<f64>,
    
    /// Maximum number of concurrent calls agent can handle
    pub max_current_calls: Option<u32>,
    
    /// Selection strategy to use
    pub strategy: SelectionStrategy,
    
    /// Whether to consider current workload in selection
    pub consider_workload: bool,
}

/// Agent selection strategies
#[derive(Debug, Clone)]
pub enum SelectionStrategy {
    /// Best skill match regardless of other factors
    BestMatch,
    /// Balance load across agents
    LoadBalanced,
    /// Optimize for performance metrics
    PerformanceOptimized,
    /// Prioritize immediate availability
    AvailabilityPriority,
    /// Custom weighted strategy
    CustomWeighted(CustomWeights),
}

/// Load balancing strategies
#[derive(Debug, Clone)]
pub enum LoadBalancingStrategy {
    /// Round-robin selection among qualified agents
    RoundRobin,
    /// Select the least busy agent
    LeastBusy,
    /// Weighted round-robin based on capacity
    WeightedRoundRobin,
    /// Random selection among qualified agents
    Random,
}

/// Result of agent selection process
#[derive(Debug, Clone)]
pub struct AgentSelectionResult {
    /// Selected agent identifier
    pub agent_id: String,
    
    /// Match score for this selection (0.0 - 1.0)
    pub match_score: f64,
    
    /// Skills that were matched
    pub matched_skills: Vec<String>,
    
    /// Skills that were not matched
    pub unmatched_skills: Vec<String>,
    
    /// Agent's current workload
    pub current_workload: u32,
    
    /// Agent's performance rating
    pub performance_rating: f64,
    
    /// Selection reasoning
    pub selection_reason: String,
}

/// Custom weights for agent selection
#[derive(Debug, Clone)]
pub struct CustomWeights {
    /// Weight for skill matching (0.0 - 1.0)
    pub skill_match: f64,
    
    /// Weight for performance rating (0.0 - 1.0)
    pub performance: f64,
    
    /// Weight for availability (0.0 - 1.0)
    pub availability: f64,
    
    /// Weight for workload (0.0 - 1.0, lower workload = higher score)
    pub workload: f64,
}

/// Performance weighting factors
#[derive(Debug, Clone)]
pub struct PerformanceWeights {
    /// Weight for customer satisfaction scores
    pub customer_satisfaction: f64,
    
    /// Weight for first call resolution rate
    pub first_call_resolution: f64,
    
    /// Weight for average handling time efficiency
    pub handling_time_efficiency: f64,
    
    /// Weight for schedule adherence
    pub schedule_adherence: f64,
}

/// Agent workload tracking information
#[derive(Debug, Clone)]
pub struct AgentWorkload {
    /// Agent identifier
    pub agent_id: String,
    
    /// Current number of active calls
    pub current_calls: u32,
    
    /// Maximum calls this agent can handle
    pub max_capacity: u32,
    
    /// Utilization percentage (0.0 - 100.0)
    pub utilization_percentage: f64,
    
    /// Last workload update timestamp
    pub last_updated: DateTime<Utc>,
}

/// Skill demand statistics
#[derive(Debug, Clone)]
pub struct SkillDemandStats {
    /// Skill name
    pub skill_name: String,
    
    /// Number of recent requests for this skill
    pub recent_requests: u32,
    
    /// Average wait time for this skill
    pub average_wait_time_seconds: u64,
    
    /// Demand trend over time
    pub demand_trend: DemandTrend,
    
    /// Last statistics update
    pub last_updated: DateTime<Utc>,
}

/// Demand trend indicators
#[derive(Debug, Clone)]
pub enum DemandTrend {
    /// Demand is increasing
    Increasing,
    /// Demand is stable
    Stable,
    /// Demand is decreasing
    Decreasing,
    /// Not enough data to determine trend
    Unknown,
}

/// Skill coverage analysis result
#[derive(Debug, Clone)]
pub struct SkillCoverageAnalysis {
    /// Statistics for each skill
    pub skill_statistics: HashMap<String, SkillCoverageStats>,
    
    /// Overall coverage summary
    pub overall_coverage: f64,
    
    /// Skills with critical low coverage
    pub critical_skills: Vec<String>,
    
    /// Analysis timestamp
    pub analyzed_at: DateTime<Utc>,
}

/// Coverage statistics for a specific skill
#[derive(Debug, Clone)]
pub struct SkillCoverageStats {
    /// Number of agents with this skill
    pub agent_count: u32,
    
    /// Coverage percentage (0.0 - 100.0)
    pub coverage_percentage: f64,
    
    /// Average proficiency level
    pub average_proficiency: f64,
    
    /// Peak demand vs. capacity ratio
    pub demand_capacity_ratio: f64,
}

/// Agent availability information by skill
#[derive(Debug, Clone)]
pub struct AgentAvailabilityInfo {
    /// Agent identifier
    pub agent_id: String,
    
    /// Whether agent is currently available
    pub is_available: bool,
    
    /// Current status
    pub status: AgentStatus,
    
    /// Skills this agent has
    pub skills: Vec<String>,
    
    /// Current workload
    pub current_workload: u32,
}

/// Configuration for agent selection behavior
#[derive(Debug, Clone)]
struct SelectionConfig {
    /// Default minimum match threshold
    default_min_match_threshold: f64,
    
    /// Whether to enable fallback to partial matches
    enable_fallback: bool,
    
    /// Maximum agents to consider in selection process
    max_agents_to_evaluate: usize,
    
    /// Cache duration for agent data in seconds
    agent_cache_duration_seconds: u64,
}

impl SkillBasedRouter {
    /// Create a new skill-based router
    ///
    /// Initializes a new skill-based router with default configuration.
    /// The router will be ready to perform agent selection immediately.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::SkillBasedRouter;
    /// 
    /// let router = SkillBasedRouter::new();
    /// println!("Skill-based router initialized");
    /// ```
    pub fn new() -> Self {
        Self {
            load_balancing_strategy: LoadBalancingStrategy::LeastBusy,
            performance_weights: PerformanceWeights {
                customer_satisfaction: 0.4,
                first_call_resolution: 0.3,
                handling_time_efficiency: 0.2,
                schedule_adherence: 0.1,
            },
            agent_workloads: HashMap::new(),
            skill_demand: HashMap::new(),
            selection_config: SelectionConfig {
                default_min_match_threshold: 0.6,
                enable_fallback: true,
                max_agents_to_evaluate: 100,
                agent_cache_duration_seconds: 60,
            },
        }
    }
    
    /// Find best agent based on skills and availability
    ///
    /// Selects the most suitable agent for a call based on required skills
    /// and current availability. Uses the configured selection strategy.
    ///
    /// # Arguments
    ///
    /// * `required_skills` - List of skills required for the call
    ///
    /// # Returns
    ///
    /// `Ok(Some(agent_id))` if a suitable agent is found, 
    /// `Ok(None)` if no suitable agent available, or error.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::SkillBasedRouter;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let router = SkillBasedRouter::new();
    /// 
    /// let skills = vec!["customer_service".to_string(), "english".to_string()];
    /// 
    /// match router.find_best_agent(&skills).await? {
    ///     Some(agent_id) => {
    ///         println!("Selected agent: {}", agent_id);
    ///     }
    ///     None => {
    ///         println!("No suitable agent available");
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn find_best_agent(&self, required_skills: &[String]) -> Result<Option<String>> {
        if required_skills.is_empty() {
            warn!("No skills specified for agent selection");
            return Ok(None);
        }
        
        // Create default criteria
        let criteria = AgentSelectionCriteria {
            required_skills: required_skills.to_vec(),
            preferred_skills: Vec::new(),
            min_performance_rating: None,
            max_current_calls: None,
            strategy: SelectionStrategy::BestMatch,
            consider_workload: true,
        };
        
        // Use the more comprehensive selection method
        match self.select_agent_with_criteria(&criteria).await? {
            Some(result) => Ok(Some(result.agent_id)),
            None => Ok(None),
        }
    }
    
    /// Select agent with detailed criteria
    ///
    /// Performs agent selection using comprehensive criteria including
    /// performance requirements, workload considerations, and selection strategy.
    ///
    /// # Arguments
    ///
    /// * `criteria` - Detailed selection criteria
    ///
    /// # Returns
    ///
    /// `Ok(Some(AgentSelectionResult))` if a suitable agent is found, 
    /// `Ok(None)` if no suitable agent available, or error.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::routing::{SkillBasedRouter, AgentSelectionCriteria, SelectionStrategy};
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let router = SkillBasedRouter::new();
    /// 
    /// let criteria = AgentSelectionCriteria {
    ///     required_skills: vec!["technical_support".to_string()],
    ///     preferred_skills: vec!["tier2".to_string()],
    ///     min_performance_rating: Some(0.85),
    ///     max_current_calls: Some(1),
    ///     strategy: SelectionStrategy::PerformanceOptimized,
    ///     consider_workload: true,
    /// };
    /// 
    /// match router.select_agent_with_criteria(&criteria).await? {
    ///     Some(result) => {
    ///         println!("Selected: {} (score: {:.2})", result.agent_id, result.match_score);
    ///     }
    ///     None => {
    ///         println!("No agent meets criteria");
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn select_agent_with_criteria(&self, criteria: &AgentSelectionCriteria) -> Result<Option<AgentSelectionResult>> {
        // TODO: Implement comprehensive agent selection logic
        // This would typically:
        // 1. Query agent registry for available agents
        // 2. Filter agents by skill requirements
        // 3. Score agents based on criteria and strategy
        // 4. Apply load balancing considerations
        // 5. Return the best match
        
        warn!("🚧 select_agent_with_criteria not fully implemented yet");
        
        // Return placeholder result for demonstration
        Ok(Some(AgentSelectionResult {
            agent_id: "agent-001".to_string(),
            match_score: 0.85,
            matched_skills: criteria.required_skills.clone(),
            unmatched_skills: Vec::new(),
            current_workload: 1,
            performance_rating: 0.9,
            selection_reason: "Best skill match with high performance".to_string(),
        }))
    }
    
    /// Select multiple agents for concurrent calls
    ///
    /// Selects multiple qualified agents for handling concurrent calls,
    /// applying load balancing to distribute workload effectively.
    ///
    /// # Arguments
    ///
    /// * `required_skills` - Skills required for the calls
    /// * `count` - Number of agents to select
    ///
    /// # Returns
    ///
    /// Vector of selected agents, potentially fewer than requested if
    /// insufficient qualified agents are available.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::SkillBasedRouter;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let router = SkillBasedRouter::new();
    /// 
    /// let skills = vec!["sales".to_string(), "english".to_string()];
    /// let agents = router.select_multiple_agents(&skills, 3).await?;
    /// 
    /// println!("Selected {} agents for concurrent calls", agents.len());
    /// for agent in agents {
    ///     println!("  {}: {} current calls", agent.agent_id, agent.current_workload);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn select_multiple_agents(&self, required_skills: &[String], count: usize) -> Result<Vec<AgentSelectionResult>> {
        let mut selected_agents = Vec::new();
        
        // For now, return placeholder results
        for i in 0..count.min(3) { // Limit to 3 for demo
            selected_agents.push(AgentSelectionResult {
                agent_id: format!("agent-{:03}", i + 1),
                match_score: 0.8 - (i as f64 * 0.05),
                matched_skills: required_skills.to_vec(),
                unmatched_skills: Vec::new(),
                current_workload: i as u32,
                performance_rating: 0.85,
                selection_reason: format!("Load balanced selection #{}", i + 1),
            });
        }
        
        warn!("🚧 select_multiple_agents not fully implemented yet");
        Ok(selected_agents)
    }
    
    /// Set load balancing strategy
    ///
    /// Configures the load balancing strategy used for agent selection.
    ///
    /// # Arguments
    ///
    /// * `strategy` - Load balancing strategy to use
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::routing::{SkillBasedRouter, LoadBalancingStrategy};
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut router = SkillBasedRouter::new();
    /// router.set_load_balancing_strategy(LoadBalancingStrategy::WeightedRoundRobin)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_load_balancing_strategy(&mut self, strategy: LoadBalancingStrategy) -> Result<()> {
        self.load_balancing_strategy = strategy;
        info!("Updated load balancing strategy: {:?}", self.load_balancing_strategy);
        Ok(())
    }
    
    /// Set performance weights
    ///
    /// Configures the weights used for performance-based agent selection.
    ///
    /// # Arguments
    ///
    /// * `weights` - Performance weighting configuration
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::routing::{SkillBasedRouter, PerformanceWeights};
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut router = SkillBasedRouter::new();
    /// 
    /// let weights = PerformanceWeights {
    ///     customer_satisfaction: 0.5,  // 50% weight
    ///     first_call_resolution: 0.3,  // 30% weight
    ///     handling_time_efficiency: 0.15, // 15% weight
    ///     schedule_adherence: 0.05,     // 5% weight
    /// };
    /// 
    /// router.set_performance_weights(weights)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_performance_weights(&mut self, weights: PerformanceWeights) -> Result<()> {
        // Validate that weights sum to approximately 1.0
        let total = weights.customer_satisfaction + weights.first_call_resolution + 
                   weights.handling_time_efficiency + weights.schedule_adherence;
        
        if (total - 1.0).abs() > 0.1 {
            warn!("Performance weights sum to {:.2}, expected ~1.0", total);
        }
        
        self.performance_weights = weights;
        info!("Updated performance weights");
        Ok(())
    }
    
    /// Update agent workload information
    ///
    /// Updates the current workload tracking for an agent.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - Agent whose workload to update
    /// * `workload` - Current workload information
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::routing::{SkillBasedRouter, AgentWorkload};
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut router = SkillBasedRouter::new();
    /// 
    /// let workload = AgentWorkload {
    ///     agent_id: "agent-001".to_string(),
    ///     current_calls: 2,
    ///     max_capacity: 4,
    ///     utilization_percentage: 50.0,
    ///     last_updated: chrono::Utc::now(),
    /// };
    /// 
    /// router.update_agent_workload("agent-001", workload).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn update_agent_workload(&mut self, agent_id: &str, workload: AgentWorkload) -> Result<()> {
        self.agent_workloads.insert(agent_id.to_string(), workload);
        debug!("Updated workload for agent: {}", agent_id);
        Ok(())
    }
    
    /// Analyze skill coverage across all agents
    ///
    /// Performs analysis of skill coverage to identify gaps and optimization opportunities.
    ///
    /// # Returns
    ///
    /// Comprehensive skill coverage analysis with statistics and recommendations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::SkillBasedRouter;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let router = SkillBasedRouter::new();
    /// 
    /// let analysis = router.analyze_skill_coverage().await?;
    /// 
    /// println!("Overall coverage: {:.1}%", analysis.overall_coverage);
    /// if !analysis.critical_skills.is_empty() {
    ///     println!("Critical skills needing attention: {:?}", analysis.critical_skills);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn analyze_skill_coverage(&self) -> Result<SkillCoverageAnalysis> {
        // TODO: Implement comprehensive skill coverage analysis
        warn!("🚧 analyze_skill_coverage not fully implemented yet");
        
        // Return placeholder analysis
        let mut skill_stats = HashMap::new();
        skill_stats.insert("customer_service".to_string(), SkillCoverageStats {
            agent_count: 15,
            coverage_percentage: 75.0,
            average_proficiency: 3.2,
            demand_capacity_ratio: 0.8,
        });
        skill_stats.insert("technical_support".to_string(), SkillCoverageStats {
            agent_count: 8,
            coverage_percentage: 40.0,
            average_proficiency: 3.8,
            demand_capacity_ratio: 1.2,
        });
        
        Ok(SkillCoverageAnalysis {
            skill_statistics: skill_stats,
            overall_coverage: 67.5,
            critical_skills: vec!["technical_support".to_string()],
            analyzed_at: Utc::now(),
        })
    }
    
    /// Get skill-based recommendations for training and hiring
    ///
    /// Analyzes current skill gaps and provides actionable recommendations.
    ///
    /// # Returns
    ///
    /// List of recommendations for improving skill coverage and agent capabilities.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::SkillBasedRouter;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let router = SkillBasedRouter::new();
    /// 
    /// let recommendations = router.get_skill_recommendations().await?;
    /// 
    /// for rec in recommendations {
    ///     println!("💡 {}", rec);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_skill_recommendations(&self) -> Result<Vec<String>> {
        // TODO: Implement intelligent recommendation system
        warn!("🚧 get_skill_recommendations not fully implemented yet");
        
        Ok(vec![
            "Consider hiring more technical support specialists".to_string(),
            "Cross-train customer service agents in basic technical skills".to_string(),
            "Increase proficiency training for tier 2 support agents".to_string(),
            "Add Spanish language capability to meet growing demand".to_string(),
        ])
    }
    
    /// Get agent availability organized by skill
    ///
    /// Returns current agent availability status organized by skill categories.
    ///
    /// # Returns
    ///
    /// Map of skills to lists of agent availability information.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::SkillBasedRouter;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let router = SkillBasedRouter::new();
    /// 
    /// let availability = router.get_agent_availability_by_skill().await?;
    /// 
    /// for (skill, agents) in availability {
    ///     let available = agents.iter().filter(|a| a.is_available).count();
    ///     println!("{}: {}/{} available", skill, available, agents.len());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_agent_availability_by_skill(&self) -> Result<HashMap<String, Vec<AgentAvailabilityInfo>>> {
        // TODO: Implement real-time availability tracking
        warn!("🚧 get_agent_availability_by_skill not fully implemented yet");
        
        // Return placeholder data
        let mut availability = HashMap::new();
        
        availability.insert("customer_service".to_string(), vec![
            AgentAvailabilityInfo {
                agent_id: "agent-001".to_string(),
                is_available: true,
                status: AgentStatus::Available,
                skills: vec!["customer_service".to_string(), "english".to_string()],
                current_workload: 0,
            },
            AgentAvailabilityInfo {
                agent_id: "agent-002".to_string(),
                is_available: false,
                status: AgentStatus::Busy(vec![]),
                skills: vec!["customer_service".to_string(), "spanish".to_string()],
                current_workload: 2,
            },
        ]);
        
        Ok(availability)
    }
    
    /// Update skill demand statistics
    ///
    /// Updates demand tracking for skills based on recent routing requests.
    ///
    /// # Arguments
    ///
    /// * `skill_name` - Skill to update demand for
    /// * `demand_increase` - Amount to increase demand tracking
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::SkillBasedRouter;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut router = SkillBasedRouter::new();
    /// 
    /// // Record increased demand for technical support
    /// router.update_skill_demand("technical_support", 3).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn update_skill_demand(&mut self, skill_name: &str, demand_increase: u32) -> Result<()> {
        let stats = self.skill_demand.entry(skill_name.to_string())
            .or_insert_with(|| SkillDemandStats {
                skill_name: skill_name.to_string(),
                recent_requests: 0,
                average_wait_time_seconds: 0,
                demand_trend: DemandTrend::Unknown,
                last_updated: Utc::now(),
            });
        
        stats.recent_requests += demand_increase;
        stats.last_updated = Utc::now();
        
        debug!("Updated demand for skill '{}': +{}", skill_name, demand_increase);
        Ok(())
    }
    
    /// Get current skill demand statistics
    ///
    /// Returns current demand statistics for all tracked skills.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::SkillBasedRouter;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let router = SkillBasedRouter::new();
    /// 
    /// let demand_stats = router.get_skill_demand_stats().await?;
    /// 
    /// for (skill, stats) in demand_stats {
    ///     println!("{}: {} recent requests, avg wait {}s", 
    ///              skill, stats.recent_requests, stats.average_wait_time_seconds);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_skill_demand_stats(&self) -> Result<HashMap<String, SkillDemandStats>> {
        Ok(self.skill_demand.clone())
    }
}

impl Default for SkillBasedRouter {
    fn default() -> Self {
        Self::new()
    }
} 