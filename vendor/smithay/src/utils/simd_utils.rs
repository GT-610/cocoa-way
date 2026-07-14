//! SIMD-Optimized Pixel Manipulation Utilities
//!
//! This module implements "Turbo-Charged" pixel format conversion.
//! It uses architecture-specific intrinsics (AVX2 for x86_64, NEON for aarch64)
//! to accelerate `wl_shm` software buffer swizzling.

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::*;

/// Swizzles a BGRA8888 buffer to RGBA8888 (or vice versa) using SIMD.
///
/// This function is optimized for high throughput "Zero-Copy" software pipelines.
/// It processes pixels in 256-bit (AVX2) or 128-bit (NEON) chunks.
pub fn swizzle_bgra_rgba(data: &mut [u8]) {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    unsafe {
        // Alignment check could be added here, but for now we assume generous alignment from shm.
        swizzle_simd(data);
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    swizzle_scalar(data);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn swizzle_simd(data: &mut [u8]) {
    let len = data.len();
    let processed = len & !31;
    let mut ptr = data.as_mut_ptr();
    let end = ptr.add(processed); // Process 32 bytes at a time

    // AVX2 Shuffle Mask for swapping R and B (0th and 2nd byte in 4-byte pixel)
    // Indices: 2, 1, 0, 3, 6, 5, 4, 7...
    let mask = _mm256_setr_epi8(
        2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15, 18, 17, 16, 19, 22, 21, 20, 23, 26, 25, 24, 27,
        30, 29, 28, 31,
    );

    while ptr < end {
        let chunk = _mm256_loadu_si256(ptr as *const __m256i);
        let swizzled = _mm256_shuffle_epi8(chunk, mask);
        _mm256_storeu_si256(ptr as *mut __m256i, swizzled);
        ptr = ptr.add(32);
    }

    swizzle_scalar(&mut data[processed..]);
}

#[cfg(target_arch = "aarch64")]
unsafe fn swizzle_simd(data: &mut [u8]) {
    let len = data.len();
    let processed = len & !15;
    let mut ptr = data.as_mut_ptr();
    let end = ptr.add(processed); // Process 16 bytes at a time (NEON is 128-bit)

    // NEON Shuffle Mask
    let mask_data: [u8; 16] = [2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15];
    let mask = vld1q_u8(mask_data.as_ptr());

    while ptr < end {
        let chunk = vld1q_u8(ptr);
        // vqtbl1q_u8 looks up bytes in 'chunk' using indices in 'mask'
        let swizzled = vqtbl1q_u8(chunk, mask);
        vst1q_u8(ptr, swizzled);
        ptr = ptr.add(16);
    }

    swizzle_scalar(&mut data[processed..]);
}

fn swizzle_scalar(data: &mut [u8]) {
    for chunk in data.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swizzle_correctness() {
        // Create a buffer with 64 pixels (enough for AVX2 and NEON paths)
        // Pattern: [R, G, B, A] = [1, 2, 3, 4]
        let mut data = vec![0u8; 64 * 4];
        for i in 0..64 {
            data[i * 4 + 0] = 1; // B (expected) / R (input)
            data[i * 4 + 1] = 2; // G
            data[i * 4 + 2] = 3; // R (expected) / B (input)
            data[i * 4 + 3] = 4; // A
        }

        swizzle_bgra_rgba(&mut data);

        for i in 0..64 {
            assert_eq!(data[i * 4 + 0], 3, "Red/Blue not swapped at pixel {}", i);
            assert_eq!(data[i * 4 + 1], 2, "Green touched at pixel {}", i);
            assert_eq!(data[i * 4 + 2], 1, "Blue/Red not swapped at pixel {}", i);
            assert_eq!(data[i * 4 + 3], 4, "Alpha touched at pixel {}", i);
        }
    }

    #[test]
    fn test_swizzle_alignment_edge_cases() {
        // Test with non-SIMD-aligned lengths (e.g. 1 pixel)
        let mut data = vec![10, 20, 30, 40];
        swizzle_bgra_rgba(&mut data);
        assert_eq!(data, vec![30, 20, 10, 40]);
    }
}
