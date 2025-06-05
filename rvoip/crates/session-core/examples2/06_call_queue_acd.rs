//! # 06 - Call Queue/ACD Server
//! 
//! An Automatic Call Distribution (ACD) server that queues incoming calls
//! and routes them to available agents. Perfect for customer service centers.

use rvoip_session_core::api::simple::*;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::{VecDeque, HashMap};
use tokio;
use std::time::{Duration, Instant};

/// Call queue/ACD server that manages agent routing
struct CallQueueAcdServer {
    call_queue: Arc<Mutex<VecDeque<QueuedCall>>>,
    agents: Arc<Mutex<HashMap<String, Agent>>>,
    queue_music: String,
}

#[derive(Debug, Clone)]
struct QueuedCall {
    caller: String,
    queue_time: Instant,
    priority: CallPriority,
}

#[derive(Debug, Clone)]
struct Agent {
    id: String,
    status: AgentStatus,
    current_call: Option<String>,
    skills: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum AgentStatus {
    Available,
    Busy,
    OnBreak,
    Offline,
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
enum CallPriority {
    Low = 1,
    Normal = 2,
    High = 3,
    Emergency = 4,
}

impl CallQueueAcdServer {
    fn new() -> Self {
        let mut agents = HashMap::new();
        
        // Add some demo agents
        agents.insert("agent1".to_string(), Agent {
            id: "agent1".to_string(),
            status: AgentStatus::Available,
            current_call: None,
            skills: vec!["general".to_string(), "billing".to_string()],
        });
        
        agents.insert("agent2".to_string(), Agent {
            id: "agent2".to_string(),
            status: AgentStatus::Available,
            current_call: None,
            skills: vec!["general".to_string(), "technical".to_string()],
        });

        Self {
            call_queue: Arc::new(Mutex::new(VecDeque::new())),
            agents: Arc::new(Mutex::new(agents)),
            queue_music: "assets/hold_music.wav".to_string(),
        }
    }

    async fn find_available_agent(&self, required_skills: &[String]) -> Option<String> {
        let agents = self.agents.lock().await;
        
        for (agent_id, agent) in agents.iter() {
            if agent.status == AgentStatus::Available {
                // Check if agent has required skills (or no specific skills required)
                if required_skills.is_empty() || 
                   required_skills.iter().any(|skill| agent.skills.contains(skill)) {
                    return Some(agent_id.clone());
                }
            }
        }
        None
    }

    async fn assign_agent(&self, agent_id: &str, caller: &str) -> bool {
        let mut agents = self.agents.lock().await;
        
        if let Some(agent) = agents.get_mut(agent_id) {
            if agent.status == AgentStatus::Available {
                agent.status = AgentStatus::Busy;
                agent.current_call = Some(caller.to_string());
                println!("👩‍💼 Agent {} assigned to caller {}", agent_id, caller);
                return true;
            }
        }
        false
    }

    async fn release_agent(&self, agent_id: &str) {
        let mut agents = self.agents.lock().await;
        
        if let Some(agent) = agents.get_mut(agent_id) {
            agent.status = AgentStatus::Available;
            agent.current_call = None;
            println!("👩‍💼 Agent {} is now available", agent_id);
        }
    }

    async fn add_to_queue(&self, caller: &str, priority: CallPriority) {
        let mut queue = self.call_queue.lock().await;
        
        let queued_call = QueuedCall {
            caller: caller.to_string(),
            queue_time: Instant::now(),
            priority,
        };

        // Insert based on priority (higher priority first)
        let mut inserted = false;
        for (i, existing_call) in queue.iter().enumerate() {
            if queued_call.priority > existing_call.priority {
                queue.insert(i, queued_call);
                inserted = true;
                break;
            }
        }
        
        if !inserted {
            queue.push_back(queued_call);
        }

        println!("📞 {} added to queue (position: {}, priority: {:?})", 
            caller, queue.len(), priority);
    }

    async fn get_queue_position(&self, caller: &str) -> Option<usize> {
        let queue = self.call_queue.lock().await;
        queue.iter().position(|call| call.caller == caller).map(|pos| pos + 1)
    }

    async fn remove_from_queue(&self, caller: &str) {
        let mut queue = self.call_queue.lock().await;
        queue.retain(|call| call.caller != caller);
    }
}

impl CallHandler for CallQueueAcdServer {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        let caller = call.from();
        println!("📞 ACD: Incoming call from {}", caller);

        // Determine call priority (could be based on caller, time of day, etc.)
        let priority = match call.get_parameter("priority") {
            Some("emergency") => CallPriority::Emergency,
            Some("high") => CallPriority::High,
            Some("low") => CallPriority::Low,
            _ => CallPriority::Normal,
        };

        // Extract required skills from call
        let required_skills: Vec<String> = call.get_parameter("skills")
            .map(|s| s.split(',').map(|skill| skill.trim().to_string()).collect())
            .unwrap_or_default();

        // Try to find an available agent immediately
        if let Some(agent_id) = self.find_available_agent(&required_skills).await {
            if self.assign_agent(&agent_id, caller).await {
                println!("✅ Direct routing: {} → Agent {}", caller, agent_id);
                return CallAction::Answer;
            }
        }

        // No agent available, add to queue
        self.add_to_queue(caller, priority).await;
        CallAction::Answer // Answer and put on hold
    }

    async fn on_call_connected(&self, call: &ActiveCall) {
        let caller = call.remote_party();
        
        // Check if caller was assigned an agent directly
        if let Some(agent_id) = self.find_assigned_agent(caller).await {
            println!("✅ Call connected: {} with Agent {}", caller, agent_id);
            call.announce(&format!("You are connected with Agent {}", agent_id)).await.ok();
        } else {
            // Caller is in queue
            if let Some(position) = self.get_queue_position(caller).await {
                println!("🎵 {} placed on hold (queue position: {})", caller, position);
                call.announce(&format!("You are number {} in queue. Please hold.")).await.ok();
                call.play_music_on_hold(&self.queue_music).await.ok();
                
                // Start queue monitoring for this caller
                self.monitor_queue_for_caller(call.clone()).await;
            }
        }
    }

    async fn on_call_ended(&self, call: &ActiveCall, reason: &str) {
        let caller = call.remote_party();
        println!("📴 ACD: Call ended with {}: {}", caller, reason);

        // Remove from queue if still queued
        self.remove_from_queue(caller).await;

        // Release agent if assigned
        if let Some(agent_id) = self.find_assigned_agent(caller).await {
            self.release_agent(&agent_id).await;
        }
    }
}

impl CallQueueAcdServer {
    async fn find_assigned_agent(&self, caller: &str) -> Option<String> {
        let agents = self.agents.lock().await;
        agents.iter()
            .find(|(_, agent)| agent.current_call.as_ref() == Some(&caller.to_string()))
            .map(|(agent_id, _)| agent_id.clone())
    }

    async fn monitor_queue_for_caller(&self, call: ActiveCall) {
        let caller = call.remote_party().to_string();
        let server = self.clone(); // Note: This would need Arc<Self> in real implementation
        
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                
                // Check if agent became available
                if let Some(agent_id) = server.find_available_agent(&[]).await {
                    if server.assign_agent(&agent_id, &caller).await {
                        server.remove_from_queue(&caller).await;
                        call.stop_music_on_hold().await.ok();
                        call.announce(&format!("Connecting you to Agent {}", agent_id)).await.ok();
                        call.transfer_to_agent(&agent_id).await.ok();
                        break;
                    }
                }

                // Update queue position
                if let Some(position) = server.get_queue_position(&caller).await {
                    call.announce(&format!("You are number {} in queue")).await.ok();
                } else {
                    // Call no longer in queue (disconnected or transferred)
                    break;
                }
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Call Queue/ACD Server");

    // Create session manager with default config
    let config = SessionConfig::default();
    let session_manager = SessionManager::new(config).await?;

    // Set our ACD handler
    session_manager.set_call_handler(Arc::new(CallQueueAcdServer::new())).await?;

    // Start listening for incoming calls
    println!("🎧 ACD server listening on 0.0.0.0:5060");
    println!("📞 Call with ?priority=high for priority routing");
    println!("📞 Call with ?skills=technical,billing for skill-based routing");
    session_manager.start_server("0.0.0.0:5060").await?;

    // Keep running
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_agent_assignment() {
        let server = CallQueueAcdServer::new();
        let agent_id = server.find_available_agent(&[]).await;
        assert!(agent_id.is_some());
        
        let assigned = server.assign_agent(&agent_id.unwrap(), "test@example.com").await;
        assert!(assigned);
    }

    #[tokio::test]
    async fn test_queue_priority() {
        let server = CallQueueAcdServer::new();
        server.add_to_queue("normal@example.com", CallPriority::Normal).await;
        server.add_to_queue("emergency@example.com", CallPriority::Emergency).await;
        
        let queue = server.call_queue.lock().await;
        assert_eq!(queue[0].caller, "emergency@example.com");
        assert_eq!(queue[1].caller, "normal@example.com");
    }
} 