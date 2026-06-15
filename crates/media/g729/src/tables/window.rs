/// Public constant `HAMMING_240`.
pub const HAMMING_240: [u16; 240] = {
    let mut arr = [0u16; 240];
    let mut i = 0;
    while i < 240 {
        if i < 120 {
            arr[i] = 2621 + (i as u16) * 246;
        } else {
            arr[i] = 2621 + ((239 - i) as u16) * 246;
        }
        i += 1;
    }
    arr
};

/// Public constant `LAG_WINDOW_12`.
pub const LAG_WINDOW_12: [u16; 12] = [
    32767, 32603, 32115, 31315, 30225, 28874, 27300, 25544, 23656, 21685, 19684, 17702,
];
