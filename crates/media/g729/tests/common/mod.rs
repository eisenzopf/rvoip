pub mod itu_format;
pub mod pcm_format;

use std::path::PathBuf;

pub fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate should be inside project root")
        .to_path_buf()
}

pub fn annex_a_vectors() -> PathBuf {
    project_root().join("reference/itu_reference_code/G729_Release3/g729AnnexA/test_vectors")
}

pub fn annex_b_vectors() -> PathBuf {
    let preferred =
        project_root().join("reference/itu_reference_code/G729_Release3/g729AnnexB/test_vectors");
    if preferred.exists() {
        preferred
    } else {
        project_root().join("reference/itu_reference_code/g729_annex_b_test_vectors")
    }
}
