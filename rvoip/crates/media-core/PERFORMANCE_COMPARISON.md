# Media-Core Performance: Debug vs Release

## Overview

Performance comparison of media-core examples between debug (unoptimized) and release (optimized) builds shows dramatic improvements with compiler optimizations enabled.

## Performance Results

### 🔇 AEC (Acoustic Echo Cancellation) Demo

| Build Mode | Time per Frame | Real-time Factor | Speed Improvement |
|------------|----------------|------------------|-------------------|
| Debug      | 626 μs         | 31.9x            | Baseline          |
| **Release**| **56 μs**      | **357.1x**       | **11.2x faster**  |

- Can process 357 seconds of audio in 1 second (release mode)
- Over 11x performance improvement with optimizations

### 🎛️ Audio Processing Pipeline Demo

| Build Mode | Avg Processing Time | Speed Improvement |
|------------|---------------------|-------------------|
| Debug      | 18.00 μs           | Baseline          |
| **Release**| **2.33 μs**        | **7.7x faster**   |

- Sub-3 microsecond processing in release mode
- Nearly 8x performance improvement

### 🎙️ Conference Mixing Demo

| Build Mode | Avg Mixing Latency | Speed Improvement |
|------------|-------------------|-------------------|
| Debug      | 24 μs             | Baseline          |
| **Release**| **1 μs**          | **24x faster**    |

- Single microsecond latency for conference mixing
- 24x performance improvement - exceptional for real-time mixing

## Key Takeaways

1. **Always benchmark in release mode** - Debug builds can be 10-25x slower

2. **Real-time performance is exceptional** - All operations are well within real-time constraints:
   - AEC: 357x faster than real-time
   - Processing pipeline: ~2.3 μs per 20ms frame (8,700x headroom)
   - Conference mixing: 1 μs latency (20,000x headroom for 20ms frames)

3. **Production ready** - The optimized performance leaves massive headroom for:
   - Multiple simultaneous sessions
   - Additional processing features
   - Lower-powered devices

## Performance Optimization Techniques Used

1. **Zero-copy audio frames** - Reduced memory allocations
2. **Object pooling** - Reuse of audio frame buffers
3. **SIMD optimizations** - Vectorized audio processing
4. **Lock-free data structures** - Minimal contention
5. **Efficient algorithms** - Optimized G.711 codecs with lookup tables

## Running Performance Tests

```bash
# Debug mode (development)
cargo run --example <example_name>

# Release mode (production)
cargo run --release --example <example_name>

# With logging (conference demo)
RUST_LOG=info cargo run --release --example conference_demo
```

## Conclusion

The media-core crate demonstrates exceptional performance in release builds, with processing times measured in microseconds rather than milliseconds. This level of performance ensures the system can handle high-density deployments with many concurrent sessions while maintaining low latency and CPU usage. 