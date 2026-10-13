//! Raw DEFLATE (RFC 1951) decoding, for the ZIP entries, and CRC-32.
//!
//! The whole input and the whole output are in memory: the archive is a few
//! megabytes. The output never grows past the size the caller names, so a
//! damaged or hostile stream cannot use up the memory.

use alloc::vec::Vec;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum InflateError {
    /// The stream ends before its last block does.
    Truncated,
    /// A block type, code table or back reference that DEFLATE does not allow.
    Invalid,
    /// More output than the caller allowed.
    TooLarge,
}

struct Bits<'a> {
    data: &'a [u8],
    position: usize,
    buffer: u32,
    count: u32,
}

impl Bits<'_> {
    fn need(&mut self, count: u32) -> Result<(), InflateError> {
        while self.count < count {
            let byte = *self
                .data
                .get(self.position)
                .ok_or(InflateError::Truncated)?;
            self.position += 1;
            self.buffer |= u32::from(byte) << self.count;
            self.count += 8;
        }
        Ok(())
    }

    /// `count` bits (at most 16), least significant first.
    fn take(&mut self, count: u32) -> Result<u32, InflateError> {
        if count == 0 {
            return Ok(0);
        }
        self.need(count)?;
        let value = self.buffer & ((1 << count) - 1);
        self.buffer >>= count;
        self.count -= count;
        Ok(value)
    }
}

const MAX_BITS: usize = 15;

/// A canonical Huffman code: how many codes each length has, and the symbols
/// in code order.
struct Huffman {
    counts: [u16; MAX_BITS + 1],
    symbols: [u16; 288],
}

impl Huffman {
    /// From the code length of each symbol. The code must use its whole
    /// space; an incomplete one is allowed only with a single code (as
    /// DEFLATE's distance table may be) or none.
    fn new(lengths: &[u8]) -> Result<Self, InflateError> {
        let mut counts = [0_u16; MAX_BITS + 1];
        for &length in lengths {
            counts[length as usize] += 1;
        }
        counts[0] = 0;
        let mut left = 1_i32;
        for &count in &counts[1..] {
            left = (left << 1) - i32::from(count);
            if left < 0 {
                return Err(InflateError::Invalid);
            }
        }
        let used: u32 = counts.iter().map(|&count| u32::from(count)).sum();
        if left > 0 && used > 1 {
            return Err(InflateError::Invalid);
        }
        let mut offsets = [0_u16; MAX_BITS + 2];
        for length in 1..=MAX_BITS {
            offsets[length + 1] = offsets[length] + counts[length];
        }
        let mut symbols = [0_u16; 288];
        for (symbol, &length) in lengths.iter().enumerate() {
            if length != 0 {
                symbols[offsets[length as usize] as usize] = symbol as u16;
                offsets[length as usize] += 1;
            }
        }
        Ok(Self { counts, symbols })
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, InflateError> {
        let (mut code, mut first, mut index) = (0_i32, 0_i32, 0_i32);
        for length in 1..=MAX_BITS {
            code |= bits.take(1)? as i32;
            let count = i32::from(self.counts[length]);
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(InflateError::Invalid)
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DISTANCE_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// The order the code-length code's own lengths come in.
const ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

fn block(
    bits: &mut Bits,
    out: &mut Vec<u8>,
    limit: usize,
    literals: &Huffman,
    distances: &Huffman,
) -> Result<(), InflateError> {
    loop {
        let symbol = literals.decode(bits)? as usize;
        match symbol {
            0..=255 => {
                if out.len() >= limit {
                    return Err(InflateError::TooLarge);
                }
                out.push(symbol as u8);
            }
            256 => return Ok(()),
            257..=285 => {
                let index = symbol - 257;
                let length = LENGTH_BASE[index] as usize
                    + bits.take(u32::from(LENGTH_EXTRA[index]))? as usize;
                let index = distances.decode(bits)? as usize;
                if index >= 30 {
                    return Err(InflateError::Invalid);
                }
                let distance = DISTANCE_BASE[index] as usize
                    + bits.take(u32::from(DISTANCE_EXTRA[index]))? as usize;
                if distance > out.len() {
                    return Err(InflateError::Invalid);
                }
                if out.len() + length > limit {
                    return Err(InflateError::TooLarge);
                }
                let start = out.len() - distance;
                // The ranges may overlap (a run), so byte by byte.
                for i in 0..length {
                    let byte = out[start + i];
                    out.push(byte);
                }
            }
            _ => return Err(InflateError::Invalid),
        }
    }
}

/// The bytes a raw DEFLATE stream stands for, at most `limit` of them.
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, InflateError> {
    let mut bits = Bits {
        data,
        position: 0,
        buffer: 0,
        count: 0,
    };
    let mut out = Vec::new();
    loop {
        let last = bits.take(1)? == 1;
        match bits.take(2)? {
            0 => {
                // Stored: to the byte boundary, then LEN and its complement.
                bits.buffer = 0;
                bits.count = 0;
                let header = data
                    .get(bits.position..bits.position + 4)
                    .ok_or(InflateError::Truncated)?;
                let length = u16::from_le_bytes([header[0], header[1]]);
                if length != !u16::from_le_bytes([header[2], header[3]]) {
                    return Err(InflateError::Invalid);
                }
                bits.position += 4;
                let length = length as usize;
                let stored = data
                    .get(bits.position..bits.position + length)
                    .ok_or(InflateError::Truncated)?;
                if out.len() + length > limit {
                    return Err(InflateError::TooLarge);
                }
                out.extend_from_slice(stored);
                bits.position += length;
            }
            1 => {
                let mut lengths = [0_u8; 288];
                for (symbol, length) in lengths.iter_mut().enumerate() {
                    *length = match symbol {
                        0..=143 => 8,
                        144..=255 => 9,
                        256..=279 => 7,
                        _ => 8,
                    };
                }
                let literals = Huffman::new(&lengths)?;
                // 32 codes of five bits; the last two are never valid distances.
                let distances = Huffman::new(&[5; 32])?;
                block(&mut bits, &mut out, limit, &literals, &distances)?;
            }
            2 => {
                let literal_count = bits.take(5)? as usize + 257;
                let distance_count = bits.take(5)? as usize + 1;
                let code_count = bits.take(4)? as usize + 4;
                if literal_count > 286 || distance_count > 30 {
                    return Err(InflateError::Invalid);
                }
                let mut code_lengths = [0_u8; 19];
                for &index in &ORDER[..code_count] {
                    code_lengths[index] = bits.take(3)? as u8;
                }
                let code = Huffman::new(&code_lengths)?;
                let mut lengths = [0_u8; 286 + 30];
                let total = literal_count + distance_count;
                let mut at = 0;
                while at < total {
                    let symbol = code.decode(&mut bits)?;
                    let (value, repeat) = match symbol {
                        0..=15 => (symbol as u8, 1),
                        16 => {
                            if at == 0 {
                                return Err(InflateError::Invalid);
                            }
                            (lengths[at - 1], 3 + bits.take(2)? as usize)
                        }
                        17 => (0, 3 + bits.take(3)? as usize),
                        18 => (0, 11 + bits.take(7)? as usize),
                        _ => return Err(InflateError::Invalid),
                    };
                    if at + repeat > total {
                        return Err(InflateError::Invalid);
                    }
                    lengths[at..at + repeat].fill(value);
                    at += repeat;
                }
                // The end-of-block code must exist.
                if lengths[256] == 0 {
                    return Err(InflateError::Invalid);
                }
                let literals = Huffman::new(&lengths[..literal_count])?;
                let distances = Huffman::new(&lengths[literal_count..total])?;
                block(&mut bits, &mut out, limit, &literals, &distances)?;
            }
            _ => return Err(InflateError::Invalid),
        }
        if last {
            return Ok(out);
        }
    }
}

/// CRC-32 (the ZIP one: polynomial 0xEDB88320), continued from `crc`
/// (0 to start).
pub fn crc32(crc: u32, data: &[u8]) -> u32 {
    let mut crc = !crc;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_of_the_check_string() {
        assert_eq!(crc32(0, b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(crc32(0, b"1234"), b"56789"), 0xCBF4_3926);
        assert_eq!(crc32(0, b""), 0);
    }

    #[test]
    fn a_stored_block() {
        // "hello" stored, as zlib writes it at level 0.
        let data = [0x01, 0x05, 0x00, 0xFA, 0xFF, b'h', b'e', b'l', b'l', b'o'];
        assert_eq!(inflate(&data, 100).unwrap(), b"hello");
        assert_eq!(inflate(&data, 4), Err(InflateError::TooLarge));
        assert_eq!(inflate(&data[..7], 100), Err(InflateError::Truncated));
        let mut bad = data;
        bad[3] = 0;
        assert_eq!(inflate(&bad, 100), Err(InflateError::Invalid));
    }

    #[test]
    fn a_fixed_block_with_a_run() {
        // zlib.compress(b"aaaaaaaaaaaaaaaaaaaa", 9)[2:-4]: a literal and a
        // back reference of distance 1 (an overlapping copy).
        let data = [0x4B, 0x4C, 0xC4, 0x04, 0x00];
        assert_eq!(inflate(&data, 100).unwrap(), [b'a'; 20]);
        assert_eq!(inflate(&data, 19), Err(InflateError::TooLarge));
    }

    #[test]
    fn nothing_and_garbage() {
        assert_eq!(inflate(&[], 10), Err(InflateError::Truncated));
        // Block type 3 does not exist.
        assert_eq!(inflate(&[0x07], 10), Err(InflateError::Invalid));
        // An empty fixed block is an empty output.
        assert_eq!(inflate(&[0x03, 0x00], 10).unwrap(), b"");
    }

    #[test]
    fn a_dynamic_block_and_a_stream_of_several_blocks() {
        let text = include_bytes!("../tests/data/text.bin");
        let dynamic = include_bytes!("../tests/data/text.dynamic.deflate");
        assert_eq!(dynamic[0] >> 1 & 3, 2, "the fixture is a dynamic block");
        assert_eq!(inflate(dynamic, text.len()).unwrap(), text);
        assert_eq!(
            inflate(dynamic, text.len() - 1),
            Err(InflateError::TooLarge)
        );
        let multi = include_bytes!("../tests/data/text.multi.deflate");
        let expected = include_bytes!("../tests/data/text.multi.bin");
        assert_eq!(inflate(multi, 1 << 20).unwrap(), expected);
    }

    #[test]
    fn a_cut_or_damaged_stream_is_an_error_and_never_a_panic() {
        let text = include_bytes!("../tests/data/text.bin");
        let dynamic = include_bytes!("../tests/data/text.dynamic.deflate");
        for cut in 0..dynamic.len() {
            assert!(
                inflate(&dynamic[..cut], text.len()).is_err(),
                "cut at {cut}"
            );
        }
        // Every single-bit change either fails or gives other bytes within
        // the limit; none may panic or run past it.
        let mut data = dynamic.to_vec();
        for at in 0..data.len() {
            for bit in 0..8 {
                data[at] ^= 1 << bit;
                if let Ok(out) = inflate(&data, text.len()) {
                    assert!(out.len() <= text.len());
                }
                data[at] ^= 1 << bit;
            }
        }
    }
}
