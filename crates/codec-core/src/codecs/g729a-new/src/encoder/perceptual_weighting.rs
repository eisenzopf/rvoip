//! Perceptual weighting filter implementation for G.729A
//!
//! This module implements the perceptual weighting filter as specified in
//! G.729A. The filter is used to shape the quantization noise according to
//! the spectral characteristics of the input signal.

use crate::common::basic_operators::*;

const GAMMA1: Word16 = 24576; // 0.75 in Q15 (from LD8A.H)
const GAMMA2: Word16 = 18022; // 0.55 in Q15 (from LD8A.H GAMMA2_PST)
const M: usize = 10;

/// Debug information for weight_az function calculations
///
/// This struct stores intermediate values and results from the weight_az
/// function for debugging and analysis purposes.
#[derive(Debug)]
pub struct WeightAzDebug {
    /// Current step in the calculation
    pub step: usize,
    /// Current gamma factor
    pub fac: Word16,
    /// Input coefficient value
    pub a_val: Word16,
    /// Intermediate multiplication result
    pub temp: Word32,
    /// Final result after rounding
    pub result: Word16,
}

/// Weight the LPC coefficients with a gamma factor
///
/// This function applies the gamma factor to the LPC coefficients to create
/// the perceptual weighting filter coefficients.
///
/// # Arguments
///
/// * `a` - Input LPC coefficients
/// * `gamma` - Gamma factor (in Q15)
/// * `ap` - Output weighted coefficients
/// * `debug_info` - Debug information about the calculation steps
pub fn weight_az(a: &[Word16], gamma: Word16, ap: &mut [Word16], debug_info: &mut Vec<WeightAzDebug>) {
    ap[0] = a[0];
    let mut fac = gamma;

    for i in 1..=M {
        let temp = l_mult(a[i], fac);
        let result = round(temp);
        ap[i] = result;

        debug_info.push(WeightAzDebug {
            step: i,
            fac,
            a_val: a[i],
            temp,
            result,
        });

        fac = mult(fac, gamma);
    }
}

/// Calculate perceptual weighting filter coefficients
///
/// This function calculates two sets of filter coefficients:
/// - P(z) with gamma1 = 0.75 (from LD8A.H)
/// - F(z) with gamma2 = 0.55 (from LD8A.H GAMMA2_PST)
///
/// # Arguments
///
/// * `a` - Input LPC coefficients
/// * `p` - Output P(z) filter coefficients
/// * `f` - Output F(z) filter coefficients
///
/// # Returns
///
/// Debug information for both P(z) and F(z) calculations
pub fn perceptual_weighting(a: &[Word16], p: &mut [Word16], f: &mut [Word16]) -> (Vec<WeightAzDebug>, Vec<WeightAzDebug>) {
    let mut p_debug = Vec::new();
    let mut f_debug = Vec::new();
    
    weight_az(&a[..=M], GAMMA1, p, &mut p_debug);
    weight_az(&a[..=M], GAMMA2, f, &mut f_debug);
    
    (p_debug, f_debug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::{self, BufRead, BufReader, Write};
    use std::path::Path;

    fn write_debug_info(prefix: &str, debug_info: &[WeightAzDebug]) -> io::Result<()> {
        let path = Path::new("tests")
            .join("perceptual_weighting")
            .join(format!("rust_{}_debug.csv", prefix));
            
        let mut file = File::create(path)?;
        writeln!(file, "step,fac,a_val,temp,result")?;
        for info in debug_info {
            writeln!(file, "{},{},{},{},{}", 
                info.step, info.fac, info.a_val, info.temp, info.result)?;
        }
        Ok(())
    }

    #[test]
    fn test_perceptual_weighting_basic() {
        let a = [4096, -2043, 10, 32, -1, -10, 5, 1, -1, 0, 0];
        let mut p = [0; 11];
        let mut f = [0; 11];

        let (p_debug, f_debug) = perceptual_weighting(&a, &mut p, &mut f);
        
        write_debug_info("basic_p", &p_debug).unwrap();
        write_debug_info("basic_f", &f_debug).unwrap();

        let expected_p = [4096, -1920, 9, 28, -1, -8, 4, 1, -1, 0, 0];
        let expected_f = [4096, -1226, 4, 11, 0, -2, 1, 0, 0, 0, 0];

        assert_eq!(p, expected_p);
        assert_eq!(f, expected_f);
    }

    #[test]
    fn test_weight_az_individual() {
        let a = [4096, 2048, 1024, 512, 256, 128, 64, 32, 16, 8, 4];
        let mut ap = [0; 11];
        let mut debug_info = Vec::new();
        
        weight_az(&a, GAMMA1, &mut ap, &mut debug_info);
        write_debug_info("weight_az_test", &debug_info).unwrap();

        // First coefficient should be unchanged
        assert_eq!(ap[0], a[0]);

        // Verify gamma scaling pattern
        let mut expected_fac = GAMMA1;
        for i in 1..=M {
            let expected_temp = l_mult(a[i], expected_fac);
            let expected_result = round(expected_temp);
            assert_eq!(ap[i], expected_result, "Mismatch at index {}", i);
            expected_fac = mult(expected_fac, GAMMA1);
        }
    }

    #[test]
    fn test_reference_compatibility() {
        // Test values from test_inputs.csv first row
        let a = [4096, -4174, 1, 17, 12, 13, 13, 13, 11, 9, 103];
        let mut p = [0; 11];
        let mut f = [0; 11];

        let (p_debug, f_debug) = perceptual_weighting(&a, &mut p, &mut f);
        
        write_debug_info("ref_p", &p_debug).unwrap();
        write_debug_info("ref_f", &f_debug).unwrap();

        // Expected values from c_output.csv first row
        let expected_p = [4096, -3924, 1, 14, 9, 10, 9, 8, 7, 5, 55];
        let expected_f = [4096, -2504, 0, 4, 2, 1, 1, 0, 0, 0, 1];

        for i in 0..11 {
            assert_eq!(p[i], expected_p[i], "P mismatch at index {}", i);
            assert_eq!(f[i], expected_f[i], "F mismatch at index {}", i);
        }
    }

    #[test]
    fn test_perceptual_weighting_from_csv() {
        let file = File::open("tests/perceptual_weighting/test_inputs.csv").expect("Failed to open test_inputs.csv");
        let reader = BufReader::new(file);
        
        println!("test_id,p0,p1,p2,p3,p4,p5,p6,p7,p8,p9,p10,f0,f1,f2,f3,f4,f5,f6,f7,f8,f9,f10");

        for (index, line) in reader.lines().enumerate() {
            if index == 0 { continue; } // Skip header

            let line = line.expect("Failed to read line");
            let values: Vec<i16> = line.split(',')
                .map(|s| s.trim().parse().expect("Failed to parse value"))
                .collect();

            let test_id = values[0];
            let a: [i16; 11] = values[1..12].try_into().expect("Slice with incorrect length");

            let mut p = [0; 11];
            let mut f = [0; 11];

            let (p_debug, f_debug) = perceptual_weighting(&a, &mut p, &mut f);
            
            write_debug_info(&format!("csv_{}_p", test_id), &p_debug).unwrap();
            write_debug_info(&format!("csv_{}_f", test_id), &f_debug).unwrap();

            print!("{}", test_id);
            for val in p.iter() { print!(",{}", val); }
            for val in f.iter() { print!(",{}", val); }
            println!();
        }
    }
}
