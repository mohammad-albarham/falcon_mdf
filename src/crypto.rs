//! Cryptographic utilities for encrypted attachments.
//!
//! Provides RFC 1321 MD5 hashing and FIPS 197 AES-256-CBC decryption
//! without external C or third-party cryptographic dependencies.

/// Computes the MD5 checksum of `data` (16 bytes).
pub fn md5_digest(data: &[u8]) -> [u8; 16] {
    let mut state = [0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut buffer = [0u8; 64];
    let mut offset = 0usize;

    // Process full 64-byte chunks
    while offset + 64 <= data.len() {
        buffer.copy_from_slice(&data[offset..offset + 64]);
        md5_transform(&mut state, &buffer);
        offset += 64;
    }

    // Prepare padding
    let rem = data.len() - offset;
    buffer[..rem].copy_from_slice(&data[offset..]);
    buffer[rem] = 0x80;
    for b in &mut buffer[rem + 1..] {
        *b = 0;
    }

    if rem >= 56 {
        md5_transform(&mut state, &buffer);
        buffer = [0u8; 64];
    }

    buffer[56..64].copy_from_slice(&bit_len.to_le_bytes());
    md5_transform(&mut state, &buffer);

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&state[0].to_le_bytes());
    out[4..8].copy_from_slice(&state[1].to_le_bytes());
    out[8..12].copy_from_slice(&state[2].to_le_bytes());
    out[12..16].copy_from_slice(&state[3].to_le_bytes());
    out
}

/// Computes the MD5 checksum as a 32-character lowercase hex string.
pub fn md5_hex(data: &[u8]) -> String {
    let digest = md5_digest(data);
    let mut hex = String::with_capacity(32);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{:02x}", b);
    }
    hex
}

#[inline(always)]
fn md5_transform(state: &mut [u32; 4], block: &[u8; 64]) {
    let mut m = [0u32; 16];
    for i in 0..16 {
        m[i] = u32::from_le_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }

    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];

    macro_rules! ff {
        ($a:expr, $b:expr, $c:expr, $d:expr, $k:expr, $s:expr, $i:expr) => {
            $a = $b.wrapping_add(
                ($a.wrapping_add(($b & $c) | ((!$b) & $d))
                    .wrapping_add(m[$k])
                    .wrapping_add($i))
                .rotate_left($s),
            );
        };
    }

    macro_rules! gg {
        ($a:expr, $b:expr, $c:expr, $d:expr, $k:expr, $s:expr, $i:expr) => {
            $a = $b.wrapping_add(
                ($a.wrapping_add(($b & $d) | ($c & (!$d)))
                    .wrapping_add(m[$k])
                    .wrapping_add($i))
                .rotate_left($s),
            );
        };
    }

    macro_rules! hh {
        ($a:expr, $b:expr, $c:expr, $d:expr, $k:expr, $s:expr, $i:expr) => {
            $a = $b.wrapping_add(
                ($a.wrapping_add($b ^ $c ^ $d)
                    .wrapping_add(m[$k])
                    .wrapping_add($i))
                .rotate_left($s),
            );
        };
    }

    macro_rules! ii {
        ($a:expr, $b:expr, $c:expr, $d:expr, $k:expr, $s:expr, $i:expr) => {
            $a = $b.wrapping_add(
                ($a.wrapping_add($c ^ ($b | (!$d)))
                    .wrapping_add(m[$k])
                    .wrapping_add($i))
                .rotate_left($s),
            );
        };
    }

    // Round 1
    ff!(a, b, c, d, 0, 7, 0xd76aa478);
    ff!(d, a, b, c, 1, 12, 0xe8c7b756);
    ff!(c, d, a, b, 2, 17, 0x242070db);
    ff!(b, c, d, a, 3, 22, 0xc1bdceee);
    ff!(a, b, c, d, 4, 7, 0xf57c0faf);
    ff!(d, a, b, c, 5, 12, 0x4787c62a);
    ff!(c, d, a, b, 6, 17, 0xa8304613);
    ff!(b, c, d, a, 7, 22, 0xfd469501);
    ff!(a, b, c, d, 8, 7, 0x698098d8);
    ff!(d, a, b, c, 9, 12, 0x8b44f7af);
    ff!(c, d, a, b, 10, 17, 0xffff5bb1);
    ff!(b, c, d, a, 11, 22, 0x895cd7be);
    ff!(a, b, c, d, 12, 7, 0x6b901122);
    ff!(d, a, b, c, 13, 12, 0xfd987193);
    ff!(c, d, a, b, 14, 17, 0xa679438e);
    ff!(b, c, d, a, 15, 22, 0x49b40821);

    // Round 2
    gg!(a, b, c, d, 1, 5, 0xf61e2562);
    gg!(d, a, b, c, 6, 9, 0xc040b340);
    gg!(c, d, a, b, 11, 14, 0x265e5a51);
    gg!(b, c, d, a, 0, 20, 0xe9b6c7aa);
    gg!(a, b, c, d, 5, 5, 0xd62f105d);
    gg!(d, a, b, c, 10, 9, 0x02441453);
    gg!(c, d, a, b, 15, 14, 0xd8a1e681);
    gg!(b, c, d, a, 4, 20, 0xe7d3fbc8);
    gg!(a, b, c, d, 9, 5, 0x21e1cde6);
    gg!(d, a, b, c, 14, 9, 0xc33707d6);
    gg!(c, d, a, b, 3, 14, 0xf4d50d87);
    gg!(b, c, d, a, 8, 20, 0x455a14ed);
    gg!(a, b, c, d, 13, 5, 0xa9e3e905);
    gg!(d, a, b, c, 2, 9, 0xfcefa3f8);
    gg!(c, d, a, b, 7, 14, 0x676f02d9);
    gg!(b, c, d, a, 12, 20, 0x8d2a4c8a);

    // Round 3
    hh!(a, b, c, d, 5, 4, 0xfffa3942);
    hh!(d, a, b, c, 8, 11, 0x8771f681);
    hh!(c, d, a, b, 11, 16, 0x6d9d6122);
    hh!(b, c, d, a, 14, 23, 0xfde5380c);
    hh!(a, b, c, d, 1, 4, 0xa4beea44);
    hh!(d, a, b, c, 4, 11, 0x4bdecfa9);
    hh!(c, d, a, b, 7, 16, 0xf6bb4b60);
    hh!(b, c, d, a, 10, 23, 0xbebfbc70);
    hh!(a, b, c, d, 13, 4, 0x289b7ec6);
    hh!(d, a, b, c, 0, 11, 0xeaa127fa);
    hh!(c, d, a, b, 3, 16, 0xd4ef3085);
    hh!(b, c, d, a, 6, 23, 0x04881d05);
    hh!(a, b, c, d, 9, 4, 0xd9d4d039);
    hh!(d, a, b, c, 12, 11, 0xe6db99e5);
    hh!(c, d, a, b, 15, 16, 0x1fa27cf8);
    hh!(b, c, d, a, 2, 23, 0xc4ac5665);

    // Round 4
    ii!(a, b, c, d, 0, 6, 0xf4292244);
    ii!(d, a, b, c, 7, 10, 0x432aff97);
    ii!(c, d, a, b, 14, 15, 0xab9423a7);
    ii!(b, c, d, a, 5, 21, 0xfc93a039);
    ii!(a, b, c, d, 12, 6, 0x655b59c3);
    ii!(d, a, b, c, 3, 10, 0x8f0ccc92);
    ii!(c, d, a, b, 10, 15, 0xffeff47d);
    ii!(b, c, d, a, 1, 21, 0x85845dd1);
    ii!(a, b, c, d, 8, 6, 0x6fa87e4f);
    ii!(d, a, b, c, 15, 10, 0xfe2ce6e0);
    ii!(c, d, a, b, 6, 15, 0xa3014314);
    ii!(b, c, d, a, 13, 21, 0x4e0811a1);
    ii!(a, b, c, d, 4, 6, 0xf7537e82);
    ii!(d, a, b, c, 11, 10, 0xbd3af235);
    ii!(c, d, a, b, 2, 15, 0x2ad7d2bb);
    ii!(b, c, d, a, 9, 21, 0xeb86d391);

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
}

// ---------------------------------------------------------------------------
// AES-256 Key Expansion and CBC Decryption
// ---------------------------------------------------------------------------

const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

const INV_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

const RCON: [u32; 10] = [
    0x01000000, 0x02000000, 0x04000000, 0x08000000, 0x10000000, 0x20000000, 0x40000000, 0x80000000,
    0x1b000000, 0x36000000,
];

#[inline(always)]
fn sub_word(w: u32) -> u32 {
    let b0 = SBOX[(w >> 24) as usize] as u32;
    let b1 = SBOX[((w >> 16) & 0xff) as usize] as u32;
    let b2 = SBOX[((w >> 8) & 0xff) as usize] as u32;
    let b3 = SBOX[(w & 0xff) as usize] as u32;
    (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
}

#[inline(always)]
fn rot_word(w: u32) -> u32 {
    w.rotate_left(8)
}

/// Expands a 256-bit (32-byte) key into 15 round keys of 16 bytes each.
pub struct Aes256Key {
    round_keys: [u32; 60],
}

impl Aes256Key {
    /// Expands the 32-byte key for AES-256.
    pub fn new(key: &[u8; 32]) -> Self {
        let mut w = [0u32; 60];
        for i in 0..8 {
            w[i] = u32::from_be_bytes([key[i * 4], key[i * 4 + 1], key[i * 4 + 2], key[i * 4 + 3]]);
        }

        for i in 8..60 {
            let mut temp = w[i - 1];
            if i % 8 == 0 {
                temp = sub_word(rot_word(temp)) ^ RCON[(i / 8) - 1];
            } else if i % 8 == 4 {
                temp = sub_word(temp);
            }
            w[i] = w[i - 8] ^ temp;
        }

        Self { round_keys: w }
    }
}

#[inline(always)]
fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    for _ in 0..8 {
        if (b & 1) != 0 {
            p ^= a;
        }
        let hi = (a & 0x80) != 0;
        a <<= 1;
        if hi {
            a ^= 0x1b;
        }
        b >>= 1;
    }
    p
}

#[inline(always)]
fn inv_mix_columns(state: &mut [u8; 16]) {
    for c in 0..4 {
        let col = c * 4;
        let s0 = state[col];
        let s1 = state[col + 1];
        let s2 = state[col + 2];
        let s3 = state[col + 3];

        state[col] = gmul(0x0e, s0) ^ gmul(0x0b, s1) ^ gmul(0x0d, s2) ^ gmul(0x09, s3);
        state[col + 1] = gmul(0x09, s0) ^ gmul(0x0e, s1) ^ gmul(0x0b, s2) ^ gmul(0x0d, s3);
        state[col + 2] = gmul(0x0d, s0) ^ gmul(0x09, s1) ^ gmul(0x0e, s2) ^ gmul(0x0b, s3);
        state[col + 3] = gmul(0x0b, s0) ^ gmul(0x0d, s1) ^ gmul(0x09, s2) ^ gmul(0x0e, s3);
    }
}

#[inline(always)]
fn inv_shift_rows(state: &mut [u8; 16]) {
    // Row 0 unchanged
    // Row 1 cyclic right shift by 1
    let temp = state[13];
    state[13] = state[9];
    state[9] = state[5];
    state[5] = state[1];
    state[1] = temp;

    // Row 2 cyclic right shift by 2
    state.swap(2, 10);
    state.swap(6, 14);

    // Row 3 cyclic right shift by 3 (left shift by 1)
    let temp = state[3];
    state[3] = state[7];
    state[7] = state[11];
    state[11] = state[15];
    state[15] = temp;
}

#[inline(always)]
fn inv_sub_bytes(state: &mut [u8; 16]) {
    for b in state.iter_mut() {
        *b = INV_SBOX[*b as usize];
    }
}

#[inline(always)]
fn add_round_key(state: &mut [u8; 16], round_key: &[u32], round: usize) {
    for i in 0..4 {
        let rk = round_key[round * 4 + i].to_be_bytes();
        state[i * 4] ^= rk[0];
        state[i * 4 + 1] ^= rk[1];
        state[i * 4 + 2] ^= rk[2];
        state[i * 4 + 3] ^= rk[3];
    }
}

/// Decrypts a single 16-byte AES-256 block.
pub fn aes256_decrypt_block(block: &[u8; 16], key: &Aes256Key) -> [u8; 16] {
    let mut state = *block;

    add_round_key(&mut state, &key.round_keys, 14);

    for round in (1..14).rev() {
        inv_shift_rows(&mut state);
        inv_sub_bytes(&mut state);
        add_round_key(&mut state, &key.round_keys, round);
        inv_mix_columns(&mut state);
    }

    inv_shift_rows(&mut state);
    inv_sub_bytes(&mut state);
    add_round_key(&mut state, &key.round_keys, 0);

    state
}

/// Decrypts ciphertext in AES-256-CBC mode.
///
/// `ciphertext` length must be a multiple of 16.
pub fn aes256_cbc_decrypt(ciphertext: &[u8], key: &[u8; 32], iv: &[u8; 16]) -> Vec<u8> {
    let aes_key = Aes256Key::new(key);
    let mut plaintext = Vec::with_capacity(ciphertext.len());
    let mut prev_block = *iv;

    for chunk in ciphertext.chunks_exact(16) {
        let block: &[u8; 16] = chunk.try_into().unwrap();
        let decrypted = aes256_decrypt_block(block, &aes_key);
        let mut xored = [0u8; 16];
        for i in 0..16 {
            xored[i] = decrypted[i] ^ prev_block[i];
        }
        plaintext.extend_from_slice(&xored);
        prev_block = *block;
    }

    plaintext
}

/// Derives a 32-byte AES-256 key from a password string/bytes.
///
/// Encodes as UTF-8, pads with zeroes up to 32 bytes or truncates to 32 bytes,
/// matching ASAM MDF / asammdf password handling.
pub fn derive_aes256_key(password: &[u8]) -> [u8; 32] {
    let mut key = [0u8; 32];
    if password.len() < 32 {
        key[..password.len()].copy_from_slice(password);
    } else {
        key.copy_from_slice(&password[..32]);
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_md5_rfc1321_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"a"), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"message digest"),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
        assert_eq!(
            md5_hex(b"abcdefghijklmnopqrstuvwxyz"),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
    }

    #[test]
    fn test_aes256_ecb_decrypt() {
        // NIST SP 800-38A vector for AES-256 ECB
        let key = [
            0x60, 0x3d, 0xeb, 0x10, 0x15, 0xca, 0x71, 0xbe, 0x2b, 0x73, 0xae, 0xf0, 0x85, 0x7d,
            0x77, 0x81, 0x1f, 0x35, 0x2c, 0x07, 0x3b, 0x61, 0x08, 0xd7, 0x2d, 0x98, 0x10, 0xa3,
            0x09, 0x14, 0xdf, 0xf4,
        ];
        let ciphertext = [
            0xf3, 0xee, 0xd1, 0xbd, 0xb5, 0xd2, 0xa0, 0x3c, 0x06, 0x4b, 0x5a, 0x7e, 0x3d, 0xb1,
            0x81, 0xf8,
        ];
        let expected_plaintext = [
            0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93,
            0x17, 0x2a,
        ];

        let aes_key = Aes256Key::new(&key);
        let decrypted = aes256_decrypt_block(&ciphertext, &aes_key);
        assert_eq!(decrypted, expected_plaintext);
    }

    #[test]
    fn test_aes256_cbc_decrypt() {
        // NIST SP 800-38A vector for AES-256 CBC
        let key = [
            0x60, 0x3d, 0xeb, 0x10, 0x15, 0xca, 0x71, 0xbe, 0x2b, 0x73, 0xae, 0xf0, 0x85, 0x7d,
            0x77, 0x81, 0x1f, 0x35, 0x2c, 0x07, 0x3b, 0x61, 0x08, 0xd7, 0x2d, 0x98, 0x10, 0xa3,
            0x09, 0x14, 0xdf, 0xf4,
        ];
        let iv = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        let ciphertext = [
            0xf5, 0x8c, 0x4c, 0x04, 0xd6, 0xe5, 0xf1, 0xba, 0x77, 0x9e, 0xab, 0xfb, 0x5f, 0x7b,
            0xfb, 0xd6,
        ];
        let expected_plaintext = [
            0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93,
            0x17, 0x2a,
        ];

        let decrypted = aes256_cbc_decrypt(&ciphertext, &key, &iv);
        assert_eq!(decrypted, expected_plaintext);
    }
}
