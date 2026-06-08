#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResilienceLayer {
    RvoipSipApi,
    LowerLibraryHardening,
    MixedRvoipSipAndLower,
    ExternalInterop,
}

#[derive(Debug, Clone, Copy)]
pub struct ResilienceStub {
    pub id: &'static str,
    pub layer: ResilienceLayer,
    pub existing_coverage: &'static str,
    pub target: &'static str,
    pub next_hardening: &'static str,
}

pub fn document_stub(stub: ResilienceStub) {
    assert!(!stub.id.is_empty(), "stub id must describe the RFC area");
    assert!(
        !stub.target.is_empty(),
        "stub target must describe the desired resilience behavior"
    );
    assert!(
        !stub.next_hardening.is_empty(),
        "stub must name the next implementation/hardening step"
    );
    eprintln!(
        "{} [{:?}]\nexisting: {}\ntarget: {}\nnext: {}",
        stub.id, stub.layer, stub.existing_coverage, stub.target, stub.next_hardening
    );
}
