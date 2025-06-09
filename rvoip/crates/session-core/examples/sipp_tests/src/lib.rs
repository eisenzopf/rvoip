//! Common utilities and types for SIPp integration tests

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use std::path::PathBuf;
use std::time::Duration;

pub mod config;

/// Test execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub scenario: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub duration: Duration,
    pub success: bool,
    pub calls_attempted: u32,
    pub calls_successful: u32,
    pub error_message: Option<String>,
    pub metrics: TestMetrics,
}

/// Performance and quality metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestMetrics {
    pub response_times: Vec<Duration>,
    pub rtp_packets_sent: u64,
    pub rtp_packets_received: u64,
    pub packet_loss_percent: f64,
    pub jitter_ms: f64,
    pub audio_quality_score: Option<f64>,
}

impl Default for TestMetrics {
    fn default() -> Self {
        Self {
            response_times: Vec::new(),
            rtp_packets_sent: 0,
            rtp_packets_received: 0,
            packet_loss_percent: 0.0,
            jitter_ms: 0.0,
            audio_quality_score: None,
        }
    }
}

/// Test scenario definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestScenario {
    pub name: String,
    pub description: String,
    pub sipp_xml_path: PathBuf,
    pub test_type: TestType,
    pub expected_calls: u32,
    pub timeout: Duration,
    pub audio_file: Option<PathBuf>,
}

/// Type of test being executed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestType {
    /// SIPp calls our Rust server
    InboundCall,
    /// Our Rust client calls SIPp
    OutboundCall,
    /// Stress testing with multiple concurrent calls
    StressTest,
    /// Audio quality verification
    AudioTest,
}

/// Call statistics for monitoring
#[derive(Debug, Clone, Default)]
pub struct CallStats {
    pub total_calls: u32,
    pub active_calls: u32,
    pub successful_calls: u32,
    pub failed_calls: u32,
    pub average_call_duration: Duration,
}

impl CallStats {
    pub fn success_rate(&self) -> f64 {
        if self.total_calls == 0 {
            0.0
        } else {
            (self.successful_calls as f64 / self.total_calls as f64) * 100.0
        }
    }
}

/// Utility functions for test execution
pub mod utils {
    use super::*;
    use tokio::time::{sleep, timeout};
    
    /// Wait for a condition to be true with timeout
    pub async fn wait_for_condition<F>(
        mut condition: F,
        timeout_duration: Duration,
        check_interval: Duration,
    ) -> Result<()>
    where
        F: FnMut() -> bool,
    {
        timeout(timeout_duration, async {
            while !condition() {
                sleep(check_interval).await;
            }
        })
        .await
        .context("Condition timeout")?;
        
        Ok(())
    }
    
    /// Generate a unique test ID
    pub fn generate_test_id() -> String {
        format!("test_{}", uuid::Uuid::new_v4().simple())
    }
    
    /// Calculate statistics from response times
    pub fn calculate_response_stats(times: &[Duration]) -> (Duration, Duration, Duration) {
        if times.is_empty() {
            return (Duration::ZERO, Duration::ZERO, Duration::ZERO);
        }
        
        let mut sorted_times = times.to_vec();
        sorted_times.sort();
        
        let sum: Duration = sorted_times.iter().sum();
        let average = sum / sorted_times.len() as u32;
        
        let median = sorted_times[sorted_times.len() / 2];
        
        let p95_index = (sorted_times.len() as f64 * 0.95) as usize;
        let p95 = sorted_times[p95_index.min(sorted_times.len() - 1)];
        
        (average, median, p95)
    }
}

/// Test report generation
pub mod report {
    use super::*;
    use std::fs;
    
    /// Generate HTML test report
    pub fn generate_html_report(results: &[TestResult], output_path: &PathBuf) -> Result<()> {
        let html = format!(
            r#"<!DOCTYPE html>
<html>
<head>
    <title>SIPp Integration Test Report</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 20px; }}
        .summary {{ background: #f0f0f0; padding: 15px; border-radius: 5px; }}
        .test {{ margin: 10px 0; padding: 10px; border: 1px solid #ddd; }}
        .success {{ background: #e8f5e8; }}
        .failure {{ background: #ffe8e8; }}
        table {{ border-collapse: collapse; width: 100%; }}
        th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
        th {{ background-color: #f2f2f2; }}
    </style>
</head>
<body>
    <h1>SIPp Integration Test Report</h1>
    <div class="summary">
        <h2>Summary</h2>
        <p>Total Tests: {}</p>
        <p>Successful: {}</p>
        <p>Failed: {}</p>
        <p>Success Rate: {:.1}%</p>
    </div>
    <h2>Test Results</h2>
    {}
</body>
</html>"#,
            results.len(),
            results.iter().filter(|r| r.success).count(),
            results.iter().filter(|r| !r.success).count(),
            if results.is_empty() { 0.0 } else {
                (results.iter().filter(|r| r.success).count() as f64 / results.len() as f64) * 100.0
            },
            generate_test_table(results)
        );
        
        fs::write(output_path, html).context("Failed to write HTML report")?;
        Ok(())
    }
    
    fn generate_test_table(results: &[TestResult]) -> String {
        let mut table = String::from("<table><tr><th>Scenario</th><th>Duration</th><th>Calls</th><th>Success Rate</th><th>Status</th></tr>");
        
        for result in results {
            let success_rate = if result.calls_attempted == 0 {
                0.0
            } else {
                (result.calls_successful as f64 / result.calls_attempted as f64) * 100.0
            };
            
            table.push_str(&format!(
                r#"<tr class="{}"><td>{}</td><td>{:.2}s</td><td>{}/{}</td><td>{:.1}%</td><td>{}</td></tr>"#,
                if result.success { "success" } else { "failure" },
                result.scenario,
                result.duration.as_secs_f64(),
                result.calls_successful,
                result.calls_attempted,
                success_rate,
                if result.success { "✅ PASS" } else { "❌ FAIL" }
            ));
        }
        
        table.push_str("</table>");
        table
    }
    
    /// Generate JUnit XML report for CI/CD integration
    pub fn generate_junit_report(results: &[TestResult], output_path: &PathBuf) -> Result<()> {
        let total_time: f64 = results.iter().map(|r| r.duration.as_secs_f64()).sum();
        let failures = results.iter().filter(|r| !r.success).count();
        
        let mut xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="SIPp Integration Tests" tests="{}" failures="{}" time="{:.3}">
"#,
            results.len(),
            failures,
            total_time
        );
        
        for result in results {
            xml.push_str(&format!(
                r#"  <testcase name="{}" time="{:.3}""#,
                result.scenario,
                result.duration.as_secs_f64()
            ));
            
            if result.success {
                xml.push_str(" />\n");
            } else {
                xml.push_str(">\n");
                xml.push_str(&format!(
                    r#"    <failure message="{}">{}</failure>
  </testcase>
"#,
                    result.error_message.as_deref().unwrap_or("Test failed"),
                    result.error_message.as_deref().unwrap_or("No error details")
                ));
            }
        }
        
        xml.push_str("</testsuite>\n");
        
        fs::write(output_path, xml).context("Failed to write JUnit report")?;
        Ok(())
    }
} 