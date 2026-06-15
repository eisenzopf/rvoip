/// Public constant `GRID_50`.
pub const GRID_50: [i16; 51] = {
    let mut a = [0i16; 51];
    let mut i = 0;
    while i < 51 {
        a[i] = -16384 + (i as i16) * 655;
        i += 1;
    }
    a
};

/// Public constant `TAB_ZONE_153`.
pub const TAB_ZONE_153: [u8; 153] = {
    let mut a = [0u8; 153];
    let mut i = 0;
    while i < 153 {
        a[i] = if i < 38 {
            0
        } else if i < 76 {
            1
        } else if i < 114 {
            2
        } else {
            3
        };
        i += 1;
    }
    a
};
