//! Pitch detection module

use std::f64::consts::{PI, TAU};

use num::{
    Complex, Float, FromPrimitive,
    traits::{ConstOne, ConstZero, NumAssign},
};

/// Reverse the bits of `n` up to the highest set bit
///
/// # Examples
///
/// ```
/// let n = 0b101010;
/// let reversed = reverse_bits(n, 6);
/// assert_eq!(reversed, 0b010101);
/// ```
#[inline(always)]
fn reverse_bits(n: usize, highest_bit: u32) -> usize {
    n.reverse_bits() >> (usize::BITS - highest_bit as u32)
}

pub fn fft<T>(mut data: Vec<Complex<T>>) -> Vec<Complex<T>>
where
    T: Float + ConstZero + ConstOne + FromPrimitive + NumAssign,
{
    let len = data.len();
    let highest_bit = len.ilog2();
    for i in 0..len {
        let reversed = reverse_bits(i, highest_bit);
        if i < reversed {
            data.swap(i, reversed);
        }
    }

    for power in 0..len.ilog2() {
        let n = 2 << power;

        let angle = -TAU / n as f64;
        let re = T::from_f64(angle.cos()).expect("cos is bounded to [-1, 1]");
        let im = T::from_f64(angle.sin()).expect("sin is bounded to [-1, 1]");
        let w_n = Complex::new(re, im);

        for i in (0..len).step_by(n) {
            let mut w = Complex::ONE;
            for j in 0..n / 2 {
                let u = data[i + j];
                let v = data[i + j + n / 2] * w;

                data[i + j] = u + v;
                data[i + j + n / 2] = u - v;
                w *= w_n;
            }
        }
    }

    data
}

#[cfg(test)]
mod tests {
    use crate::fft::reverse_bits;

    #[test]
    fn test_reverse_bits() {
        let n: usize = 0b100000;
        let highest_bit = n.ilog2() - 1;
        let reversed = reverse_bits(n, highest_bit);
        assert_eq!(reversed, 0b000001);
        let reversed = reverse_bits(reversed, highest_bit);
        assert_eq!(reversed, 0b100000);
    }
}
