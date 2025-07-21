pub type Word16 = i16;
pub type Word32 = i32;

pub fn L_mult(var1: Word16, var2: Word16) -> Word32 {
    (var1 as Word32) * (var2 as Word32)
}

pub fn L_mac(L_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    L_var3.saturating_add(L_mult(var1, var2))
}

pub fn round(L_var1: Word32) -> Word16 {
    let y = (L_var1.saturating_add(0x8000)) >> 16;
    if y > std::i16::MAX as i32 {
        std::i16::MAX
    } else if y < std::i16::MIN as i32 {
        std::i16::MIN
    } else {
        y as Word16
    }
}

pub fn shl(var1: Word16, var2: Word16) -> Word16 {
    if var2 < 0 {
        return shr(var1, -var2);
    }
    let val = (var1 as i32) << var2;
    if val > std::i16::MAX as i32 {
        std::i16::MAX
    } else if val < std::i16::MIN as i32 {
        std::i16::MIN
    } else {
        val as Word16
    }
}

pub fn shr(var1: Word16, var2: Word16) -> Word16 {
    if var2 < 0 {
        return shl(var1, -var2);
    }
    var1 >> var2
}

pub fn L_shl(L_var1: Word32, var2: Word16) -> Word32 {
    if var2 < 0 {
        return L_shr(L_var1, -var2);
    }
    let val = (L_var1 as i64) << var2;
    if val > std::i32::MAX as i64 {
        std::i32::MAX
    } else if val < std::i32::MIN as i64 {
        std::i32::MIN
    } else {
        val as Word32
    }
}

pub fn L_shr(L_var1: Word32, var2: Word16) -> Word32 {
    if var2 < 0 {
        return L_shl(L_var1, -var2);
    }
    L_var1 >> var2
}
