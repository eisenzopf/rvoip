use std::path::Path;

pub fn read_pcm_file(path: &Path) -> Vec<i16> {
    let data =
        std::fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
    let chunks = data.chunks_exact(2);
    assert!(
        chunks.remainder().is_empty(),
        "PCM file must contain whole i16 samples"
    );
    chunks
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect()
}

pub fn compare_pcm(
    reference: &[i16],
    test: &[i16],
    name: &str,
) -> (usize, usize, i32, Option<usize>) {
    let len = reference.len().min(test.len());
    let mut matches = 0usize;
    let mut max_error = 0i32;
    let mut first_diverge = None;

    for i in 0..len {
        let diff = (i32::from(reference[i]) - i32::from(test[i])).abs();
        if diff == 0 {
            matches += 1;
        } else {
            if first_diverge.is_none() {
                first_diverge = Some(i);
            }
            if diff > max_error {
                max_error = diff;
            }
        }
    }

    if reference.len() != test.len() {
        eprintln!(
            "{}: length mismatch ref={} test={}",
            name,
            reference.len(),
            test.len()
        );
    }

    (matches, len, max_error, first_diverge)
}
