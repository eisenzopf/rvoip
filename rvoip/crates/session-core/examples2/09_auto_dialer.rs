//! # 09 - Auto-Dialer
//! 
//! An auto-dialer that automatically dials a list of phone numbers.
//! Perfect for telemarketing, notifications, and automated outreach.

use rvoip_session_core::api::simple::*;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

/// Auto-dialer that manages a queue of calls to make
struct AutoDialer {
    session_manager: SessionManager,
    local_uri: String,
    call_queue: Arc<Mutex<VecDeque<CallTarget>>>,
    config: DialerConfig,
    stats: Arc<Mutex<DialerStats>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CallTarget {
    number: String,
    name: Option<String>,
    campaign_id: Option<String>,
    retry_count: u32,
    max_retries: u32,
    priority: CallPriority,
}

#[derive(Debug, Clone)]
struct DialerConfig {
    max_concurrent_calls: usize,
    call_timeout_seconds: u64,
    retry_delay_seconds: u64,
    pause_between_calls_ms: u64,
    predictive_dialing: bool,
}

#[derive(Debug, Clone)]
struct DialerStats {
    total_calls_attempted: u32,
    successful_connections: u32,
    busy_signals: u32,
    no_answers: u32,
    rejections: u32,
    errors: u32,
    start_time: Instant,
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
enum CallPriority {
    Low = 1,
    Normal = 2,
    High = 3,
    Emergency = 4,
}

impl AutoDialer {
    async fn new(local_uri: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let config = SessionConfig::default();
        let session_manager = SessionManager::new(config).await?;

        let dialer_config = DialerConfig {
            max_concurrent_calls: 5,
            call_timeout_seconds: 30,
            retry_delay_seconds: 300, // 5 minutes
            pause_between_calls_ms: 1000, // 1 second
            predictive_dialing: false,
        };

        let stats = DialerStats {
            total_calls_attempted: 0,
            successful_connections: 0,
            busy_signals: 0,
            no_answers: 0,
            rejections: 0,
            errors: 0,
            start_time: Instant::now(),
        };

        Ok(Self {
            session_manager,
            local_uri: local_uri.to_string(),
            call_queue: Arc::new(Mutex::new(VecDeque::new())),
            config: dialer_config,
            stats: Arc::new(Mutex::new(stats)),
        })
    }

    async fn load_call_list(&self, numbers: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
        let mut queue = self.call_queue.lock().await;
        
        for number in numbers {
            let target = CallTarget {
                number: number.clone(),
                name: None,
                campaign_id: Some("default".to_string()),
                retry_count: 0,
                max_retries: 2,
                priority: CallPriority::Normal,
            };
            queue.push_back(target);
        }

        println!("📋 Loaded {} numbers into dialer queue", queue.len());
        Ok(())
    }

    async fn load_from_csv(&self, csv_path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let content = tokio::fs::read_to_string(csv_path).await?;
        let mut numbers = Vec::new();

        for line in content.lines().skip(1) { // Skip header
            let fields: Vec<&str> = line.split(',').collect();
            if !fields.is_empty() {
                let number = fields[0].trim().to_string();
                if !number.is_empty() {
                    numbers.push(number);
                }
            }
        }

        self.load_call_list(numbers).await?;
        Ok(())
    }

    async fn start_dialing(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("🚀 Starting auto-dialer");
        println!("📊 Max concurrent calls: {}", self.config.max_concurrent_calls);

        let mut active_calls = Vec::new();

        loop {
            // Clean up completed calls
            active_calls.retain(|call: &ActiveCall| !call.is_completed());

            // Check if we can make more calls
            if active_calls.len() < self.config.max_concurrent_calls {
                if let Some(target) = self.get_next_target().await {
                    match self.make_call(&target).await {
                        Ok(call) => {
                            active_calls.push(call);
                            self.update_stats(|stats| stats.total_calls_attempted += 1).await;
                        }
                        Err(e) => {
                            println!("❌ Failed to make call to {}: {}", target.number, e);
                            self.update_stats(|stats| stats.errors += 1).await;
                            
                            // Retry logic
                            if target.retry_count < target.max_retries {
                                self.schedule_retry(target).await;
                            }
                        }
                    }
                }
            }

            // Check if we're done
            if active_calls.is_empty() && self.is_queue_empty().await {
                break;
            }

            // Pause between call attempts
            tokio::time::sleep(Duration::from_millis(self.config.pause_between_calls_ms)).await;
        }

        self.print_final_stats().await;
        println!("✅ Auto-dialer completed");
        Ok(())
    }

    async fn make_call(&self, target: &CallTarget) -> Result<ActiveCall, Box<dyn std::error::Error>> {
        println!("📞 Dialing {} ({})", target.number, 
            target.name.as_deref().unwrap_or("Unknown"));

        let call = self.session_manager
            .make_call(&self.local_uri, &target.number, None)
            .await?;

        // Set up call event handlers
        let stats = self.stats.clone();
        let target_number = target.number.clone();

        call.on_answered(move |_call| {
            let stats = stats.clone();
            let number = target_number.clone();
            async move {
                println!("✅ Call answered by {}", number);
                let mut stats = stats.lock().await;
                stats.successful_connections += 1;
            }
        }).await;

        call.on_busy(move |_call| {
            let stats = stats.clone();
            let number = target_number.clone();
            async move {
                println!("📞 Busy signal from {}", number);
                let mut stats = stats.lock().await;
                stats.busy_signals += 1;
            }
        }).await;

        call.on_no_answer(move |_call| {
            let stats = stats.clone();
            let number = target_number.clone();
            async move {
                println!("🔇 No answer from {}", number);
                let mut stats = stats.lock().await;
                stats.no_answers += 1;
            }
        }).await;

        call.on_rejected(move |_call, reason| {
            let stats = stats.clone();
            let number = target_number.clone();
            async move {
                println!("🚫 Call rejected by {}: {}", number, reason);
                let mut stats = stats.lock().await;
                stats.rejections += 1;
            }
        }).await;

        // Set timeout for the call
        let call_clone = call.clone();
        let timeout_duration = Duration::from_secs(self.config.call_timeout_seconds);
        tokio::spawn(async move {
            tokio::time::sleep(timeout_duration).await;
            if !call_clone.is_completed() {
                call_clone.hangup("Timeout").await.ok();
            }
        });

        Ok(call)
    }

    async fn get_next_target(&self) -> Option<CallTarget> {
        let mut queue = self.call_queue.lock().await;
        queue.pop_front()
    }

    async fn schedule_retry(&self, mut target: CallTarget) {
        target.retry_count += 1;
        
        let queue = self.call_queue.clone();
        let delay = Duration::from_secs(self.config.retry_delay_seconds);
        
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let mut queue = queue.lock().await;
            queue.push_back(target);
        });
    }

    async fn is_queue_empty(&self) -> bool {
        let queue = self.call_queue.lock().await;
        queue.is_empty()
    }

    async fn update_stats<F>(&self, updater: F) 
    where 
        F: FnOnce(&mut DialerStats),
    {
        let mut stats = self.stats.lock().await;
        updater(&mut stats);
    }

    async fn print_final_stats(&self) {
        let stats = self.stats.lock().await;
        let duration = stats.start_time.elapsed();
        
        println!("\n📊 Final Dialer Statistics:");
        println!("⏱️  Total runtime: {:?}", duration);
        println!("📞 Total calls attempted: {}", stats.total_calls_attempted);
        println!("✅ Successful connections: {}", stats.successful_connections);
        println!("📞 Busy signals: {}", stats.busy_signals);
        println!("🔇 No answers: {}", stats.no_answers);
        println!("🚫 Rejections: {}", stats.rejections);
        println!("❌ Errors: {}", stats.errors);
        
        if stats.total_calls_attempted > 0 {
            let success_rate = (stats.successful_connections as f64 / stats.total_calls_attempted as f64) * 100.0;
            println!("📈 Success rate: {:.1}%", success_rate);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Auto-Dialer");

    let args: Vec<String> = std::env::args().collect();
    let local_uri = args.get(1)
        .cloned()
        .unwrap_or_else(|| "sip:autodialer@localhost".to_string());

    let dialer = AutoDialer::new(&local_uri).await?;

    // Load phone numbers
    if let Some(csv_path) = args.get(2) {
        println!("📋 Loading numbers from CSV file: {}", csv_path);
        dialer.load_from_csv(csv_path).await?;
    } else {
        // Use demo numbers
        let demo_numbers = vec![
            "sip:target1@example.com".to_string(),
            "sip:target2@example.com".to_string(),
            "sip:target3@example.com".to_string(),
            "sip:+15551234567@provider.com".to_string(),
            "sip:+15551234568@provider.com".to_string(),
        ];
        println!("📋 Using demo numbers");
        dialer.load_call_list(demo_numbers).await?;
    }

    // Start dialing
    dialer.start_dialing().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_auto_dialer_creation() {
        let dialer = AutoDialer::new("sip:test@localhost").await;
        assert!(dialer.is_ok());
    }

    #[tokio::test]
    async fn test_call_queue() {
        let dialer = AutoDialer::new("sip:test@localhost").await.unwrap();
        let numbers = vec!["123".to_string(), "456".to_string()];
        
        dialer.load_call_list(numbers).await.unwrap();
        
        let target = dialer.get_next_target().await;
        assert!(target.is_some());
        assert_eq!(target.unwrap().number, "123");
    }
} 