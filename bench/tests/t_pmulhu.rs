use std::arch::x86_64::*;

fn main() {
    unsafe {
        let a = _mm_setr_epi16(
            0,
            1,
            2,
            3,
            65535u16 as i16,
            32768u16 as i16,
            12345,
            60000u16 as i16,
        );
        let b = _mm_setr_epi16(
            65535u16 as i16,
            2,
            3,
            4,
            65535u16 as i16,
            65535u16 as i16,
            2,
            2,
        );
        let r = _mm_mulhi_epu16(a, b);
        let mut lanes = [0u16; 8];
        _mm_storeu_si128(lanes.as_mut_ptr() as *mut __m128i, r);
        for (i, lane) in lanes.iter().enumerate() {
            if i != 0 {
                print!(" ");
            }
            print!("{lane}");
        }
        println!();
    }
}
