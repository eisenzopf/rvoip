#!/bin/bash

# Fix compilation issues in codec-core

echo "Fixing compilation issues in codec-core..."

# Fix the missing SilkDecoder and CeltDecoder types
cat > crates/codec-core/src/codecs/opus_fix.rs << 'EOF'
// Add missing decoder types
#[derive(Debug, Clone)]
pub struct SilkDecoder {
    state: SilkDecoderState,
    plc: PacketLossConcealment,
}

#[derive(Debug, Clone)]
pub struct CeltDecoder {
    mdct: MdctTransform,
    quantizer: CeltQuantizer,
    bit_allocation: BitAllocation,
}

#[derive(Debug, Clone)]
struct SilkDecoderState;

impl SilkDecoder {
    pub fn new(_sample_rate: u32, _channels: u8) -> Self {
        Self {
            state: SilkDecoderState,
            plc: PacketLossConcealment::new(),
        }
    }
    
    pub fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        // Simplified SILK decoding simulation
        let mut samples = Vec::with_capacity(data.len() * 8);
        
        for &byte in data {
            // Expand each byte to multiple samples
            for i in 0..8 {
                let sample = ((byte as i16) << 8) | (i * 256);
                samples.push(sample);
            }
        }
        
        Ok(samples)
    }
}

impl CeltDecoder {
    pub fn new(_sample_rate: u32, _channels: u8) -> Self {
        Self {
            mdct: MdctTransform::new(),
            quantizer: CeltQuantizer::new(),
            bit_allocation: BitAllocation::new(),
        }
    }
    
    pub fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        // Simplified CELT decoding simulation
        let mut samples = Vec::with_capacity(data.len() * 4);
        
        for &byte in data {
            // Expand each byte to multiple samples
            for i in 0..4 {
                let sample = ((byte as i16) << 8) | (i * 256);
                samples.push(sample);
            }
        }
        
        Ok(samples)
    }
}

impl SilkDecoderState {
    fn new() -> Self {
        Self {}
    }
}
EOF

# Fix the μ-law decoder issues
cat > crates/codec-core/src/utils/simd_fix.rs << 'EOF'
/// Fixed μ-law to linear conversion
pub fn mulaw_to_linear_scalar(mulaw: u8) -> i16 {
    const BIAS: i16 = 0x84;
    const MULAW_MAX: u8 = 0x7F;
    
    let mulaw = mulaw ^ MULAW_MAX;
    let sign = mulaw & 0x80;
    let exponent = (mulaw >> 4) & 0x07;
    let mantissa = mulaw & 0x0F;
    
    let mut sample = ((mantissa as i16) << (exponent + 3)) + BIAS;
    
    if exponent > 0 {
        sample += 1i16 << (exponent + 2);
    }
    
    if sign != 0 {
        -sample
    } else {
        sample
    }
}
EOF

echo "Compilation fixes applied!" 