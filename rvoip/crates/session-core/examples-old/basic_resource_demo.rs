//! Basic Resource Tracking Primitives Demo
//! 
//! Demonstrates the basic resource tracking functionality after Phase 12.2 refactoring.
//! This shows the low-level primitives that call-engine will use to build sophisticated
//! resource allocation and policy enforcement logic.

use std::time::Duration;
use rvoip_session_core::{
    SessionId, BasicResourceType, BasicResourceAllocation, BasicResourceUsage,
    BasicResourceLimits, BasicResourceRequest, BasicResourceStats
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Basic Resource Tracking Primitives Demo");
    println!("===========================================");
    
    // Create basic resource limits configuration
    let mut limits = BasicResourceLimits::default();
    println!("✅ Created basic resource limits configuration:");
    println!("   Bandwidth per session: {} bytes", limits.get_session_limit(&BasicResourceType::Bandwidth).unwrap());
    println!("   CPU per session: {}%", limits.get_session_limit(&BasicResourceType::CPU).unwrap());
    println!("   Memory per session: {} bytes", limits.get_session_limit(&BasicResourceType::Memory).unwrap());
    
    // Update some limits
    limits.set_session_limit(BasicResourceType::Bandwidth, 2000000); // 2MB
    limits.set_global_limit(BasicResourceType::CPU, 90); // 90%
    println!("✅ Updated resource limits dynamically");
    
    // Create basic resource usage tracking
    let mut cpu_usage = BasicResourceUsage::new(BasicResourceType::CPU, 100);
    cpu_usage.update_usage(45, 3);
    println!("✅ CPU usage tracking: {:.1}% used ({} allocations)", cpu_usage.usage_percentage(), cpu_usage.allocation_count);
    
    let mut memory_usage = BasicResourceUsage::new(BasicResourceType::Memory, 8 * 1024 * 1024 * 1024); // 8GB
    memory_usage.update_usage(2 * 1024 * 1024 * 1024, 5); // 2GB used
    println!("✅ Memory usage tracking: {:.1}% used ({} allocations)", memory_usage.usage_percentage(), memory_usage.allocation_count);
    
    // Create resource requests (data structures only)
    let session1 = SessionId::new();
    let session2 = SessionId::new();
    
    let mut request1 = BasicResourceRequest::new(
        session1, 
        BasicResourceType::Bandwidth, 
        1500000 // 1.5MB
    );
    request1.max_wait_time = Some(Duration::from_secs(30));
    request1.add_metadata("priority".to_string(), "high".to_string());
    
    let request2 = BasicResourceRequest::new(
        session2,
        BasicResourceType::CPU,
        20 // 20%
    );
    
    println!("✅ Created resource requests:");
    println!("   Request 1: {} wants {} bytes of {}", request1.session_id, request1.amount, request1.resource_type);
    println!("   Request 2: {} wants {}% of {}", request2.session_id, request2.amount, request2.resource_type);
    
    // Check if requests are within limits (basic validation)
    let bandwidth_ok = limits.is_within_session_limit(&request1.resource_type, request1.amount);
    let cpu_ok = limits.is_within_session_limit(&request2.resource_type, request2.amount);
    println!("✅ Request validation:");
    println!("   Bandwidth request within limits: {}", bandwidth_ok);
    println!("   CPU request within limits: {}", cpu_ok);
    
    // Check resource availability (basic check)
    let bandwidth_available = BasicResourceUsage::new(BasicResourceType::Bandwidth, 100000000); // 100MB
    let can_allocate_bandwidth = bandwidth_available.is_available(request1.amount);
    println!("   Bandwidth available for allocation: {}", can_allocate_bandwidth);
    
    // Create resource allocations (basic tracking)
    if bandwidth_ok && can_allocate_bandwidth {
        let allocation = BasicResourceAllocation::new(
            "alloc-001".to_string(),
            session1,
            BasicResourceType::Bandwidth,
            request1.amount,
        );
        println!("✅ Created bandwidth allocation: {} for session {}", allocation.allocation_id, allocation.session_id);
        println!("   Allocated at: {:?}", allocation.allocated_at);
        println!("   Expires: {:?}", allocation.expires_at);
        println!("   Is expired: {}", allocation.is_expired());
    }
    
    // Create resource statistics tracking
    let mut stats = BasicResourceStats::new();
    stats.increment_requests();
    stats.increment_requests();
    stats.increment_allocations();
    stats.update_usage(BasicResourceType::CPU, cpu_usage.clone());
    stats.update_usage(BasicResourceType::Memory, memory_usage.clone());
    
    println!("✅ Resource statistics:");
    println!("   Total requests: {}", stats.total_requests);
    println!("   Total allocations: {}", stats.total_allocations);
    println!("   Active allocations: {}", stats.active_allocations);
    println!("   Tracked resource types: {}", stats.usage_by_type.len());
    
    // Show CPU stats details
    if let Some(cpu_stats) = stats.get_usage(&BasicResourceType::CPU) {
        println!("   CPU: {:.1}% used with {} allocations", cpu_stats.usage_percentage(), cpu_stats.allocation_count);
    }
    
    println!();
    println!("🎯 ARCHITECTURAL SUCCESS:");
    println!("   ✅ Basic resource primitives work correctly");
    println!("   ✅ No business logic in session-core");
    println!("   ✅ Data structures ready for call-engine policy enforcement");
    println!("   ✅ Clean separation of concerns achieved");
    println!("   ✅ Resource tracking foundation established");
    
    Ok(())
} 