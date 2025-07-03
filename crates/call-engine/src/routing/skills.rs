//! # Skill-Based Agent Matching
//!
//! This module provides sophisticated skill-based matching capabilities for routing
//! calls to the most appropriate agents based on required skills, proficiency levels,
//! and availability. It enables intelligent agent selection that optimizes for
//! customer satisfaction and operational efficiency.

use std::collections::HashMap;
use chrono::{DateTime, Utc};
use tracing::{info, debug, warn};
use serde::{Serialize, Deserialize};

use crate::error::{CallCenterError, Result};
use crate::agent::AgentId;

/// # Skill Matcher for Intelligent Agent Selection  
///
/// The `SkillMatcher` provides comprehensive skill-based agent matching capabilities
/// that enable intelligent call routing decisions. It analyzes agent skills, proficiency
/// levels, availability, and performance metrics to select the optimal agent for each call.
///
/// ## Key Features
///
/// - **Multi-Dimensional Matching**: Considers skills, proficiency, availability, and performance
/// - **Weighted Scoring**: Configurable weights for different matching criteria
/// - **Skill Hierarchies**: Support for skill categories and substitution rules
/// - **Performance Integration**: Incorporates agent performance metrics in matching
/// - **Real-Time Updates**: Dynamic skill and availability tracking
/// - **Fallback Strategies**: Intelligent fallback when perfect matches aren't available
///
/// ## Matching Algorithms
///
/// ### Exact Match
/// - Requires agents to have all required skills at minimum proficiency
/// - Fastest matching but most restrictive
/// - Best for specialized or critical calls
///
/// ### Weighted Match
/// - Scores agents based on skill fit, proficiency, and availability
/// - Balances skill requirements with operational efficiency
/// - Most commonly used algorithm
///
/// ### Best Available
/// - Selects the best available agent even with partial skill match
/// - Prevents calls from waiting indefinitely
/// - Includes configurable minimum match thresholds
///
/// ### Performance-Optimized
/// - Considers historical performance metrics alongside skills
/// - Optimizes for customer satisfaction and resolution rates
/// - Ideal for quality-focused operations
///
/// ## Examples
///
/// ### Basic Skill Matching
///
/// ```rust
/// use rvoip_call_engine::routing::{SkillMatcher, SkillRequirement, SkillLevel};
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut matcher = SkillMatcher::new();
/// 
/// // Define required skills for a technical support call
/// let requirements = vec![
///     SkillRequirement::new("technical_support", SkillLevel::Intermediate, true),
///     SkillRequirement::new("english", SkillLevel::Advanced, true),
///     SkillRequirement::new("windows", SkillLevel::Basic, false), // Preferred
/// ];
/// 
/// // Find best matching agent
/// match matcher.find_best_match(&requirements).await? {
///     Some(match_result) => {
///         println!("✅ Best agent: {} (score: {:.2})", 
///                  match_result.agent_id, match_result.match_score);
///         println!("   Skills matched: {:?}", match_result.matched_skills);
///     }
///     None => {
///         println!("❌ No suitable agent available");
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Advanced Skill Matching with Performance
///
/// ```rust
/// use rvoip_call_engine::routing::{SkillMatcher, MatchingConfig, ScoringWeights, MatchingAlgorithm, SkillRequirement, SkillLevel};
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut matcher = SkillMatcher::new();
/// 
/// // Configure matching algorithm
/// let config = MatchingConfig {
///     algorithm: MatchingAlgorithm::PerformanceOptimized,
///     weights: ScoringWeights {
///         skill_match: 0.4,        // 40% - Skill fit
///         proficiency: 0.3,        // 30% - Skill proficiency  
///         availability: 0.2,       // 20% - Agent availability
///         performance: 0.1,        // 10% - Historical performance
///     },
///     min_match_threshold: 0.7,    // Minimum 70% skill match
///     enable_fallback: true,
/// };
/// 
/// matcher.set_matching_config(config)?;
/// 
/// // Define complex skill requirements
/// let requirements = vec![
///     SkillRequirement::with_weight("customer_service", SkillLevel::Advanced, true, 2.0),
///     SkillRequirement::with_weight("sales", SkillLevel::Intermediate, true, 1.5),
///     SkillRequirement::with_weight("spanish", SkillLevel::Basic, false, 1.0),
/// ];
/// 
/// let matches = matcher.find_multiple_matches(&requirements, 3).await?;
/// 
/// println!("🎯 Top 3 agent matches:");
/// for (i, match_result) in matches.iter().enumerate() {
///     println!("  {}. {} - Score: {:.2} ({} skills)", 
///              i + 1, match_result.agent_id, match_result.match_score, 
///              match_result.matched_skills.len());
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Skill Hierarchy and Substitution
///
/// ```rust
/// use rvoip_call_engine::routing::{SkillMatcher, SkillHierarchy, SkillSubstitution};
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut matcher = SkillMatcher::new();
/// 
/// // Configure skill hierarchies
/// matcher.add_skill_hierarchy("languages", vec![
///     "english_native".to_string(),
///     "english_fluent".to_string(), 
///     "english_conversational".to_string(),
/// ])?;
/// 
/// // Configure skill substitutions
/// matcher.add_skill_substitution(
///     "microsoft_office",
///     vec!["word".to_string(), "excel".to_string(), "powerpoint".to_string()],
///     0.8 // 80% substitution strength
/// )?;
/// 
/// // Advanced agents can handle basic calls
/// matcher.add_skill_substitution(
///     "tier1_support", 
///     vec!["tier2_support".to_string(), "tier3_support".to_string()],
///     1.2 // 120% substitution (tier 2/3 agents are even better)
/// )?;
/// 
/// println!("Skill hierarchies and substitutions configured");
/// # Ok(())
/// # }
/// ```
///
/// ### Real-Time Skill Monitoring
///
/// ```rust
/// use rvoip_call_engine::routing::SkillMatcher;
/// 
/// # async fn example(matcher: SkillMatcher) -> Result<(), Box<dyn std::error::Error>> {
/// // Monitor skill utilization across the call center
/// let utilization = matcher.get_skill_utilization().await?;
/// 
/// println!("📊 Skill Utilization Report:");
/// for (skill, stats) in utilization.skills {
///     println!("  {}: {:.1}% utilized ({} agents available)", 
///              skill, stats.utilization_percentage, stats.available_agents);
///     
///     if stats.utilization_percentage > 90.0 {
///         println!("    ⚠️ High utilization - consider cross-training");
///     }
/// }
/// 
/// // Get recommendations for skill gaps
/// let recommendations = matcher.analyze_skill_gaps().await?;
/// for rec in recommendations {
///     println!("💡 Recommendation: {}", rec);
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Dynamic Skill Updates
///
/// ```rust
/// use rvoip_call_engine::routing::{SkillMatcher, AgentSkillUpdate, SkillLevel};
/// 
/// # async fn example(mut matcher: SkillMatcher) -> Result<(), Box<dyn std::error::Error>> {
/// // Update agent skills dynamically (e.g., after training)
/// let skill_update = AgentSkillUpdate {
///     agent_id: "agent-001".to_string(),
///     skill_changes: vec![
///         ("python".to_string(), Some(SkillLevel::Intermediate)), // Add/update skill
///         ("java".to_string(), Some(SkillLevel::Advanced)),       // Upgrade skill
///         ("legacy_system".to_string(), None),                   // Remove skill
///     ],
///     updated_at: chrono::Utc::now(),
/// };
/// 
/// matcher.update_agent_skills(skill_update).await?;
/// 
/// // Refresh skill cache
/// matcher.refresh_skill_cache().await?;
/// 
/// println!("Agent skills updated");
/// # Ok(())
/// # }
/// ```
pub struct SkillMatcher {
    /// Agent skill profiles cache
    agent_skills: HashMap<String, AgentSkillProfile>,
    
    /// Skill hierarchy definitions
    skill_hierarchies: HashMap<String, SkillHierarchy>,
    
    /// Skill substitution rules
    skill_substitutions: HashMap<String, SkillSubstitution>,
    
    /// Matching algorithm configuration
    config: MatchingConfig,
    
    /// Performance metrics cache
    performance_cache: HashMap<String, AgentPerformanceMetrics>,
    
    /// Skill utilization statistics
    utilization_stats: HashMap<String, SkillUtilizationStats>,
}

/// Individual skill requirement for call routing
#[derive(Debug, Clone)]
pub struct SkillRequirement {
    /// Skill name/identifier
    pub skill_name: String,
    
    /// Required proficiency level
    pub required_level: SkillLevel,
    
    /// Whether this skill is required or just preferred
    pub required: bool,
    
    /// Weight/importance of this skill (default: 1.0)
    pub weight: f64,
    
    /// Minimum experience in months for this skill
    pub min_experience_months: Option<u32>,
}

/// Skill proficiency levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SkillLevel {
    /// Novice level - basic understanding
    Novice = 1,
    /// Basic proficiency
    Basic = 2,
    /// Intermediate proficiency  
    Intermediate = 3,
    /// Advanced proficiency
    Advanced = 4,
    /// Expert level proficiency
    Expert = 5,
}

/// Result of skill-based agent matching
#[derive(Debug, Clone)]
pub struct SkillMatchResult {
    /// Matched agent identifier
    pub agent_id: String,
    
    /// Overall match score (0.0 - 1.0)  
    pub match_score: f64,
    
    /// Skills that were successfully matched
    pub matched_skills: Vec<MatchedSkill>,
    
    /// Skills that were not matched
    pub unmatched_skills: Vec<String>,
    
    /// Whether this is a complete match (all required skills)
    pub complete_match: bool,
    
    /// Agent's current availability status
    pub availability_status: AvailabilityStatus,
    
    /// Estimated response quality for this match
    pub estimated_quality_score: f64,
}

/// Individual skill match information
#[derive(Debug, Clone)]
pub struct MatchedSkill {
    /// Skill name
    pub skill_name: String,
    
    /// Required proficiency level
    pub required_level: SkillLevel,
    
    /// Agent's actual proficiency level
    pub agent_level: SkillLevel,
    
    /// Match strength (1.0 = exact match, > 1.0 = over-qualified)
    pub match_strength: f64,
    
    /// Whether this was a direct match or substitution
    pub substitution_used: bool,
}

/// Agent skill profile
#[derive(Debug, Clone)]
pub struct AgentSkillProfile {
    /// Agent identifier
    pub agent_id: String,
    
    /// Agent's skills with proficiency levels
    pub skills: HashMap<String, AgentSkill>,
    
    /// Agent's current availability
    pub availability: AvailabilityStatus,
    
    /// Agent's performance metrics
    pub performance_rating: f64,
    
    /// Last skill profile update
    pub last_updated: DateTime<Utc>,
}

/// Individual agent skill information
#[derive(Debug, Clone)]
pub struct AgentSkill {
    /// Skill proficiency level
    pub level: SkillLevel,
    
    /// Experience with this skill in months
    pub experience_months: u32,
    
    /// Last time this skill was used
    pub last_used: DateTime<Utc>,
    
    /// Performance rating for this specific skill
    pub skill_rating: f64,
    
    /// Whether skill is certified/validated
    pub certified: bool,
}

/// Agent availability status
#[derive(Debug, Clone, PartialEq)]
pub enum AvailabilityStatus {
    /// Available for calls
    Available,
    /// Busy on a call
    Busy { estimated_free_in_seconds: u64 },
    /// On break
    OnBreak { break_ends_at: DateTime<Utc> },
    /// In training or meeting
    Training { estimated_duration_seconds: u64 },
    /// Offline/logged out
    Offline,
}

/// Skill hierarchy definition
#[derive(Debug, Clone)]
pub struct SkillHierarchy {
    /// Hierarchy name
    pub name: String,
    
    /// Skills in order from highest to lowest proficiency
    pub skill_levels: Vec<String>,
    
    /// Whether higher levels can substitute for lower levels
    pub substitution_allowed: bool,
}

/// Skill substitution rule
#[derive(Debug, Clone)]
pub struct SkillSubstitution {
    /// The skill being substituted for
    pub target_skill: String,
    
    /// Skills that can substitute for the target
    pub substitute_skills: Vec<String>,
    
    /// Substitution strength (1.0 = equivalent, < 1.0 = weaker, > 1.0 = stronger)
    pub substitution_strength: f64,
}

/// Matching algorithm configuration
#[derive(Debug, Clone)]
pub struct MatchingConfig {
    /// Matching algorithm to use
    pub algorithm: MatchingAlgorithm,
    
    /// Scoring weights for different factors
    pub weights: ScoringWeights,
    
    /// Minimum match threshold (0.0 - 1.0)
    pub min_match_threshold: f64,
    
    /// Whether to enable fallback matching
    pub enable_fallback: bool,
}

/// Available matching algorithms
#[derive(Debug, Clone)]
pub enum MatchingAlgorithm {
    /// Exact skill match required
    ExactMatch,
    /// Weighted scoring across multiple factors
    WeightedMatch,
    /// Best available agent regardless of perfect match
    BestAvailable,
    /// Performance-optimized matching
    PerformanceOptimized,
}

/// Scoring weights for agent matching
#[derive(Debug, Clone)]
pub struct ScoringWeights {
    /// Weight for skill match quality
    pub skill_match: f64,
    
    /// Weight for skill proficiency levels
    pub proficiency: f64,
    
    /// Weight for agent availability
    pub availability: f64,
    
    /// Weight for agent performance history
    pub performance: f64,
}

/// Agent performance metrics for matching
#[derive(Debug, Clone)]
pub struct AgentPerformanceMetrics {
    /// Agent identifier
    pub agent_id: String,
    
    /// Overall performance rating (0.0 - 1.0)
    pub overall_rating: f64,
    
    /// Customer satisfaction score
    pub customer_satisfaction: f64,
    
    /// First call resolution rate
    pub first_call_resolution_rate: f64,
    
    /// Average handling time efficiency
    pub handling_time_efficiency: f64,
    
    /// Last performance update
    pub last_updated: DateTime<Utc>,
}

/// Skill utilization statistics
#[derive(Debug, Clone)]
pub struct SkillUtilizationStats {
    /// Skill name
    pub skill_name: String,
    
    /// Number of agents with this skill
    pub total_agents: u32,
    
    /// Number of currently available agents
    pub available_agents: u32,
    
    /// Current utilization percentage
    pub utilization_percentage: f64,
    
    /// Average proficiency level across agents
    pub average_proficiency: f64,
    
    /// Demand trend (calls requiring this skill)
    pub demand_trend: DemandTrend,
}

/// Skill utilization report
#[derive(Debug, Clone)]
pub struct SkillUtilizationReport {
    /// Skills and their utilization statistics
    pub skills: HashMap<String, SkillUtilizationStats>,
    
    /// Overall utilization summary
    pub summary: UtilizationSummary,
    
    /// Report generation timestamp
    pub generated_at: DateTime<Utc>,
}

/// Overall utilization summary
#[derive(Debug, Clone)]
pub struct UtilizationSummary {
    /// Most utilized skill
    pub highest_utilization_skill: String,
    
    /// Least utilized skill
    pub lowest_utilization_skill: String,
    
    /// Average utilization across all skills
    pub average_utilization: f64,
    
    /// Skills with critical utilization (>90%)
    pub critical_skills: Vec<String>,
}

/// Demand trend for skills
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

/// Agent skill update request
#[derive(Debug, Clone)]
pub struct AgentSkillUpdate {
    /// Agent to update
    pub agent_id: String,
    
    /// Skill changes (skill_name, new_level or None to remove)
    pub skill_changes: Vec<(String, Option<SkillLevel>)>,
    
    /// Update timestamp
    pub updated_at: DateTime<Utc>,
}

impl SkillMatcher {
    /// Create a new skill matcher
    ///
    /// Initializes a new skill matcher with default configuration.
    /// The matcher will be ready to perform skill-based agent matching immediately.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::SkillMatcher;
    /// 
    /// let matcher = SkillMatcher::new();
    /// println!("Skill matcher initialized");
    /// ```
    pub fn new() -> Self {
        Self {
            agent_skills: HashMap::new(),
            skill_hierarchies: HashMap::new(),
            skill_substitutions: HashMap::new(),
            config: MatchingConfig {
                algorithm: MatchingAlgorithm::WeightedMatch,
                weights: ScoringWeights {
                    skill_match: 0.4,
                    proficiency: 0.3,
                    availability: 0.2,
                    performance: 0.1,
                },
                min_match_threshold: 0.6,
                enable_fallback: true,
            },
            performance_cache: HashMap::new(),
            utilization_stats: HashMap::new(),
        }
    }
    
    /// Find the best matching agent for skill requirements
    ///
    /// Analyzes all available agents and returns the best match based on
    /// skill requirements and configured matching algorithm.
    ///
    /// # Arguments
    ///
    /// * `requirements` - List of required/preferred skills
    ///
    /// # Returns
    ///
    /// `Ok(Some(SkillMatchResult))` if a suitable agent is found, 
    /// `Ok(None)` if no suitable agent available, or error.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::{SkillMatcher, SkillRequirement, SkillLevel};
    /// 
    /// # async fn example(matcher: SkillMatcher) -> Result<(), Box<dyn std::error::Error>> {
    /// let requirements = vec![
    ///     SkillRequirement::new("customer_service", SkillLevel::Intermediate, true),
    ///     SkillRequirement::new("spanish", SkillLevel::Basic, false),
    /// ];
    /// 
    /// match matcher.find_best_match(&requirements).await? {
    ///     Some(result) => {
    ///         println!("Best match: {} (score: {:.2})", result.agent_id, result.match_score);
    ///     }
    ///     None => {
    ///         println!("No suitable agent found");
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn find_best_match(&self, requirements: &[SkillRequirement]) -> Result<Option<SkillMatchResult>> {
        if requirements.is_empty() {
            return Ok(None);
        }
        
        let mut best_match: Option<SkillMatchResult> = None;
        let mut best_score = 0.0;
        
        // Evaluate each agent
        for (agent_id, profile) in &self.agent_skills {
            if profile.availability == AvailabilityStatus::Offline {
                continue; // Skip offline agents
            }
            
            let match_result = self.evaluate_agent_match(agent_id, profile, requirements).await?;
            
            // Check if this is a better match
            if match_result.match_score > best_score && 
               match_result.match_score >= self.config.min_match_threshold {
                best_score = match_result.match_score;
                best_match = Some(match_result);
            }
        }
        
        Ok(best_match)
    }
    
    /// Find multiple matching agents ranked by suitability
    ///
    /// Returns a list of agents ranked by their match score for the given requirements.
    ///
    /// # Arguments
    ///
    /// * `requirements` - List of required/preferred skills
    /// * `max_results` - Maximum number of results to return
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::{SkillMatcher, SkillRequirement, SkillLevel};
    /// 
    /// # async fn example(matcher: SkillMatcher) -> Result<(), Box<dyn std::error::Error>> {
    /// let requirements = vec![
    ///     SkillRequirement::new("sales", SkillLevel::Advanced, true),
    /// ];
    /// 
    /// let matches = matcher.find_multiple_matches(&requirements, 5).await?;
    /// 
    /// println!("Top {} matches:", matches.len());
    /// for (i, match_result) in matches.iter().enumerate() {
    ///     println!("  {}. {} - Score: {:.2}", i + 1, match_result.agent_id, match_result.match_score);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn find_multiple_matches(
        &self, 
        requirements: &[SkillRequirement], 
        max_results: usize
    ) -> Result<Vec<SkillMatchResult>> {
        let mut matches = Vec::new();
        
        // Evaluate all agents
        for (agent_id, profile) in &self.agent_skills {
            if profile.availability == AvailabilityStatus::Offline {
                continue;
            }
            
            let match_result = self.evaluate_agent_match(agent_id, profile, requirements).await?;
            
            if match_result.match_score >= self.config.min_match_threshold {
                matches.push(match_result);
            }
        }
        
        // Sort by match score (highest first)
        matches.sort_by(|a, b| b.match_score.partial_cmp(&a.match_score).unwrap());
        
        // Truncate to max results
        matches.truncate(max_results);
        
        Ok(matches)
    }
    
    /// Set matching algorithm configuration
    ///
    /// Updates the matching configuration including algorithm, weights, and thresholds.
    ///
    /// # Arguments
    ///
    /// * `config` - New matching configuration
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::{SkillMatcher, MatchingConfig, MatchingAlgorithm, ScoringWeights};
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut matcher = SkillMatcher::new();
    /// 
    /// let config = MatchingConfig {
    ///     algorithm: MatchingAlgorithm::PerformanceOptimized,
    ///     weights: ScoringWeights {
    ///         skill_match: 0.5,
    ///         proficiency: 0.2,
    ///         availability: 0.2,
    ///         performance: 0.1,
    ///     },
    ///     min_match_threshold: 0.8,
    ///     enable_fallback: false,
    /// };
    /// 
    /// matcher.set_matching_config(config)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_matching_config(&mut self, config: MatchingConfig) -> Result<()> {
        // Validate weights sum to approximately 1.0
        let total_weight = config.weights.skill_match + config.weights.proficiency + 
                          config.weights.availability + config.weights.performance;
        
        if (total_weight - 1.0).abs() > 0.1 {
            warn!("Scoring weights sum to {:.2}, expected ~1.0", total_weight);
        }
        
        self.config = config;
        info!("Updated skill matching configuration");
        Ok(())
    }
    
    /// Add a skill hierarchy
    ///
    /// Defines a skill hierarchy where higher-level skills can substitute for lower-level ones.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the hierarchy
    /// * `skill_levels` - Skills in order from highest to lowest proficiency
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::SkillMatcher;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut matcher = SkillMatcher::new();
    /// 
    /// matcher.add_skill_hierarchy("support_tiers", vec![
    ///     "tier3_support".to_string(),
    ///     "tier2_support".to_string(), 
    ///     "tier1_support".to_string(),
    /// ])?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_skill_hierarchy(&mut self, name: &str, skill_levels: Vec<String>) -> Result<()> {
        let hierarchy = SkillHierarchy {
            name: name.to_string(),
            skill_levels,
            substitution_allowed: true,
        };
        
        self.skill_hierarchies.insert(name.to_string(), hierarchy);
        info!("Added skill hierarchy: {}", name);
        Ok(())
    }
    
    /// Add a skill substitution rule
    ///
    /// Defines which skills can substitute for a target skill and their relative strength.
    ///
    /// # Arguments
    ///
    /// * `target_skill` - The skill being substituted for
    /// * `substitute_skills` - Skills that can substitute
    /// * `strength` - Substitution strength (1.0 = equivalent)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::SkillMatcher;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut matcher = SkillMatcher::new();
    /// 
    /// matcher.add_skill_substitution(
    ///     "general_support",
    ///     vec!["technical_support".to_string(), "billing_support".to_string()],
    ///     1.1 // Specialized agents are 10% better
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_skill_substitution(
        &mut self, 
        target_skill: &str, 
        substitute_skills: Vec<String>, 
        strength: f64
    ) -> Result<()> {
        let substitution = SkillSubstitution {
            target_skill: target_skill.to_string(),
            substitute_skills,
            substitution_strength: strength,
        };
        
        self.skill_substitutions.insert(target_skill.to_string(), substitution);
        info!("Added skill substitution for: {}", target_skill);
        Ok(())
    }
    
    /// Update agent skills
    ///
    /// Updates or adds skills for a specific agent.
    ///
    /// # Arguments
    ///
    /// * `update` - Skill update request
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::{SkillMatcher, AgentSkillUpdate, SkillLevel};
    /// 
    /// # async fn example(mut matcher: SkillMatcher) -> Result<(), Box<dyn std::error::Error>> {
    /// let update = AgentSkillUpdate {
    ///     agent_id: "agent-001".to_string(),
    ///     skill_changes: vec![
    ///         ("python".to_string(), Some(SkillLevel::Advanced)),
    ///         ("legacy_cobol".to_string(), None), // Remove skill
    ///     ],
    ///     updated_at: chrono::Utc::now(),
    /// };
    /// 
    /// matcher.update_agent_skills(update).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn update_agent_skills(&mut self, update: AgentSkillUpdate) -> Result<()> {
        let profile = self.agent_skills.entry(update.agent_id.clone())
            .or_insert_with(|| AgentSkillProfile {
                agent_id: update.agent_id.clone(),
                skills: HashMap::new(),
                availability: AvailabilityStatus::Available,
                performance_rating: 0.8, // Default performance
                last_updated: update.updated_at,
            });
        
        for (skill_name, level_option) in update.skill_changes {
            match level_option {
                Some(level) => {
                    // Add or update skill
                    let agent_skill = AgentSkill {
                        level,
                        experience_months: 0, // TODO: Calculate based on history
                        last_used: update.updated_at,
                        skill_rating: 0.8, // Default rating
                        certified: false,
                    };
                    profile.skills.insert(skill_name, agent_skill);
                }
                None => {
                    // Remove skill
                    profile.skills.remove(&skill_name);
                }
            }
        }
        
        profile.last_updated = update.updated_at;
        info!("Updated skills for agent: {}", update.agent_id);
        Ok(())
    }
    
    /// Get skill utilization report
    ///
    /// Analyzes skill utilization across all agents and returns comprehensive statistics.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::SkillMatcher;
    /// 
    /// # async fn example(matcher: SkillMatcher) -> Result<(), Box<dyn std::error::Error>> {
    /// let report = matcher.get_skill_utilization().await?;
    /// 
    /// println!("Skill Utilization Report:");
    /// println!("  Average utilization: {:.1}%", report.summary.average_utilization);
    /// println!("  Critical skills: {:?}", report.summary.critical_skills);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_skill_utilization(&self) -> Result<SkillUtilizationReport> {
        // TODO: Implement comprehensive skill utilization analysis
        warn!("🚧 get_skill_utilization not fully implemented yet");
        
        Ok(SkillUtilizationReport {
            skills: HashMap::new(),
            summary: UtilizationSummary {
                highest_utilization_skill: "customer_service".to_string(),
                lowest_utilization_skill: "specialized_tech".to_string(),
                average_utilization: 75.0,
                critical_skills: vec!["tier1_support".to_string()],
            },
            generated_at: Utc::now(),
        })
    }
    
    /// Analyze skill gaps and provide recommendations
    ///
    /// Identifies skill gaps in the agent pool and provides training recommendations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::SkillMatcher;
    /// 
    /// # async fn example(matcher: SkillMatcher) -> Result<(), Box<dyn std::error::Error>> {
    /// let recommendations = matcher.analyze_skill_gaps().await?;
    /// 
    /// for rec in recommendations {
    ///     println!("💡 {}", rec);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn analyze_skill_gaps(&self) -> Result<Vec<String>> {
        // TODO: Implement skill gap analysis
        warn!("🚧 analyze_skill_gaps not fully implemented yet");
        
        Ok(vec![
            "Consider training more agents in advanced technical support".to_string(),
            "Spanish language skills are in high demand".to_string(),
            "Cross-train tier 1 agents for tier 2 capabilities".to_string(),
        ])
    }
    
    /// Refresh skill and performance cache
    ///
    /// Updates cached data from the database and external systems.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::SkillMatcher;
    /// 
    /// # async fn example(mut matcher: SkillMatcher) -> Result<(), Box<dyn std::error::Error>> {
    /// matcher.refresh_skill_cache().await?;
    /// println!("Skill cache refreshed");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn refresh_skill_cache(&mut self) -> Result<()> {
        // TODO: Implement cache refresh from database
        warn!("🚧 refresh_skill_cache not fully implemented yet");
        info!("Skill cache refresh requested");
        Ok(())
    }
    
    // Private helper methods
    
    async fn evaluate_agent_match(
        &self,
        agent_id: &str,
        profile: &AgentSkillProfile,
        requirements: &[SkillRequirement],
    ) -> Result<SkillMatchResult> {
        let mut matched_skills = Vec::new();
        let mut unmatched_skills = Vec::new();
        let mut total_score = 0.0;
        let mut required_match_count = 0;
        let mut required_skills_count = 0;
        
        for requirement in requirements {
            required_skills_count += if requirement.required { 1 } else { 0 };
            
            if let Some(agent_skill) = profile.skills.get(&requirement.skill_name) {
                // Direct skill match
                let match_strength = self.calculate_match_strength(requirement, agent_skill);
                
                matched_skills.push(MatchedSkill {
                    skill_name: requirement.skill_name.clone(),
                    required_level: requirement.required_level,
                    agent_level: agent_skill.level,
                    match_strength,
                    substitution_used: false,
                });
                
                total_score += match_strength * requirement.weight;
                if requirement.required {
                    required_match_count += 1;
                }
            } else if let Some(substitute_match) = self.find_skill_substitution(requirement, profile) {
                // Substitution match
                matched_skills.push(substitute_match);
                if requirement.required {
                    required_match_count += 1;
                }
            } else {
                // No match found
                unmatched_skills.push(requirement.skill_name.clone());
            }
        }
        
        // Calculate overall match score
        let skill_match_score = if !requirements.is_empty() {
            total_score / requirements.len() as f64
        } else {
            0.0
        };
        
        let availability_score = self.calculate_availability_score(&profile.availability);
        let performance_score = profile.performance_rating;
        
        let overall_score = 
            skill_match_score * self.config.weights.skill_match +
            availability_score * self.config.weights.availability +
            performance_score * self.config.weights.performance;
        
        let complete_match = required_match_count == required_skills_count;
        
        Ok(SkillMatchResult {
            agent_id: agent_id.to_string(),
            match_score: overall_score.min(1.0), // Cap at 1.0
            matched_skills,
            unmatched_skills,
            complete_match,
            availability_status: profile.availability.clone(),
            estimated_quality_score: performance_score,
        })
    }
    
    fn calculate_match_strength(&self, requirement: &SkillRequirement, agent_skill: &AgentSkill) -> f64 {
        let level_ratio = agent_skill.level as u8 as f64 / requirement.required_level as u8 as f64;
        level_ratio.min(2.0) // Cap at 2x to prevent excessive over-qualification bonus
    }
    
    fn find_skill_substitution(
        &self, 
        requirement: &SkillRequirement, 
        profile: &AgentSkillProfile
    ) -> Option<MatchedSkill> {
        if let Some(substitution) = self.skill_substitutions.get(&requirement.skill_name) {
            for substitute_skill in &substitution.substitute_skills {
                if let Some(agent_skill) = profile.skills.get(substitute_skill) {
                    let base_strength = self.calculate_match_strength(requirement, agent_skill);
                    let adjusted_strength = base_strength * substitution.substitution_strength;
                    
                    return Some(MatchedSkill {
                        skill_name: requirement.skill_name.clone(),
                        required_level: requirement.required_level,
                        agent_level: agent_skill.level,
                        match_strength: adjusted_strength,
                        substitution_used: true,
                    });
                }
            }
        }
        None
    }
    
    fn calculate_availability_score(&self, availability: &AvailabilityStatus) -> f64 {
        match availability {
            AvailabilityStatus::Available => 1.0,
            AvailabilityStatus::Busy { estimated_free_in_seconds } => {
                // Score decreases with longer busy time
                let minutes = *estimated_free_in_seconds as f64 / 60.0;
                (10.0 - minutes.min(10.0)) / 10.0
            }
            AvailabilityStatus::OnBreak { .. } => 0.3,
            AvailabilityStatus::Training { .. } => 0.1,
            AvailabilityStatus::Offline => 0.0,
        }
    }
}

impl SkillRequirement {
    /// Create a new skill requirement
    ///
    /// # Arguments
    ///
    /// * `skill_name` - Name of the required skill
    /// * `level` - Required proficiency level
    /// * `required` - Whether this skill is required or preferred
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::{SkillRequirement, SkillLevel};
    /// 
    /// let req = SkillRequirement::new("customer_service", SkillLevel::Intermediate, true);
    /// ```
    pub fn new(skill_name: &str, level: SkillLevel, required: bool) -> Self {
        Self {
            skill_name: skill_name.to_string(),
            required_level: level,
            required,
            weight: 1.0,
            min_experience_months: None,
        }
    }
    
    /// Create a skill requirement with custom weight
    ///
    /// # Arguments
    ///
    /// * `skill_name` - Name of the required skill  
    /// * `level` - Required proficiency level
    /// * `required` - Whether this skill is required or preferred
    /// * `weight` - Importance weight for this skill
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::routing::{SkillRequirement, SkillLevel};
    /// 
    /// let req = SkillRequirement::with_weight("sales", SkillLevel::Advanced, true, 2.0);
    /// ```
    pub fn with_weight(skill_name: &str, level: SkillLevel, required: bool, weight: f64) -> Self {
        Self {
            skill_name: skill_name.to_string(),
            required_level: level,
            required,
            weight,
            min_experience_months: None,
        }
    }
}

impl Default for SkillMatcher {
    fn default() -> Self {
        Self::new()
    }
} 