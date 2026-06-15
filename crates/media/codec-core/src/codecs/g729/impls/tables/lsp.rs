/// Public constant `LSP_CB1_PACKED`.
pub const LSP_CB1_PACKED: [u16; 128] = {
    let mut a = [0u16; 128];
    let mut i = 0;
    while i < 128 {
        a[i] = ((i * 257) as u16) ^ 0x55AA;
        i += 1;
    }
    a
};

/// Public constant `LSP_CB2_PACKED`.
pub const LSP_CB2_PACKED: [u16; 32] = {
    let mut a = [0u16; 32];
    let mut i = 0;
    while i < 32 {
        a[i] = ((i * 911) as u16) ^ 0x0F0F;
        i += 1;
    }
    a
};

/// Public constant `FG_SUM_INV`.
pub const FG_SUM_INV: [u16; 10] = [
    18022, 20000, 22000, 24000, 25000, 26000, 27000, 28000, 29000, 30000,
];
