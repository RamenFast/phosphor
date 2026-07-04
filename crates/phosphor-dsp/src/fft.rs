// SPDX-License-Identifier: GPL-3.0-or-later
//! Iterative radix-2 FFT with precomputed tables, ported verbatim from
//! core/src/lib.rs — the spectrum family's magnitudes must stay bit-close
//! to the native-v3 fixtures, so arithmetic and accumulation order are
//! untouched.

use std::f32::consts::PI;

pub struct Fft {
    pub size: usize,
    bit_reversed: Vec<usize>,
    cosines: Vec<f32>,
    sines: Vec<f32>,
    hann: Vec<f32>,
}

impl Fft {
    pub fn new(size: usize) -> Fft {
        assert!(size.is_power_of_two());
        let levels = size.trailing_zeros();
        let bit_reversed = (0..size)
            .map(|i| i.reverse_bits() >> (usize::BITS - levels))
            .collect();
        let cosines = (0..size / 2)
            .map(|i| (2.0 * PI * i as f32 / size as f32).cos())
            .collect();
        let sines = (0..size / 2)
            .map(|i| (2.0 * PI * i as f32 / size as f32).sin())
            .collect();
        // numpy.hanning: symmetric window, denominator N-1
        let hann = (0..size)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (size as f32 - 1.0)).cos())
            .collect();
        Fft { size, bit_reversed, cosines, sines, hann }
    }

    /// Hann-windowed magnitude spectrum; `samples.len() == size`, output
    /// holds bins 0..=size/2 (the rfft layout the Python side uses).
    pub fn magnitudes(&self, samples: &[f32], out: &mut Vec<f32>) {
        let size = self.size;
        let mut real = vec![0.0f32; size];
        let mut imaginary = vec![0.0f32; size];
        for i in 0..size {
            real[i] = samples[self.bit_reversed[i]] * self.hann[self.bit_reversed[i]];
        }
        let mut half_block = 1;
        while half_block < size {
            let table_step = size / (half_block * 2);
            let mut block_start = 0;
            while block_start < size {
                let mut table_index = 0;
                for position in block_start..block_start + half_block {
                    let partner = position + half_block;
                    let cosine = self.cosines[table_index];
                    let sine = self.sines[table_index];
                    let real_product = real[partner] * cosine + imaginary[partner] * sine;
                    let imaginary_product =
                        imaginary[partner] * cosine - real[partner] * sine;
                    real[partner] = real[position] - real_product;
                    imaginary[partner] = imaginary[position] - imaginary_product;
                    real[position] += real_product;
                    imaginary[position] += imaginary_product;
                    table_index += table_step;
                }
                block_start += half_block * 2;
            }
            half_block *= 2;
        }
        out.clear();
        out.extend((0..=size / 2).map(|i| real[i].hypot(imaginary[i])));
    }
}
