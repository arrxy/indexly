//! Simple9 packs small unsigned integers into little-endian 32-bit words.
//! The high four bits select a packing mode; the low 28 bits hold its values.
//! An external value count distinguishes real values from final-word padding.

use super::IntegerCodec;
use std::io;
/// Stateless codec for values below 2^28, including zero.
pub struct Simple9;

// (values per word, bits per value), ordered from densest packing to widest values.
const MODES: [(usize, u32); 9] = [
    (28, 1),
    (14, 2),
    (9, 3),
    (7, 4),
    (5, 5),
    (4, 7),
    (3, 9),
    (2, 14),
    (1, 28),
];

impl IntegerCodec for Simple9 {
    fn id(&self) -> u8 {
        1
    }

    fn encode(&self, values: &[u32]) -> io::Result<Vec<u8>> {
        let mut output = Vec::new();
        let mut position = 0;
        while position < values.len() {
            let mut packed = false;
            // Greedily use the first mode that fits this prefix; the tail may be partial.
            for (selector, &(capacity, bits)) in MODES.iter().enumerate() {
                let count = capacity.min(values.len() - position);
                let group = &values[position..position + count];
                if group.iter().any(|&value| value >= (1_u32 << bits)) {
                    continue;
                }
                // Values occupy the low bits; unused payload bits remain zero.
                let mut word = (selector as u32) << 28;
                for (i, &value) in group.iter().enumerate() {
                    word |= value << (i as u32 * bits);
                }
                output.extend_from_slice(&word.to_le_bytes());
                position += count;
                packed = true;
                break;
            }
            if !packed {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Value too large to encode with Simple9",
                ));
            }
        }
        Ok(output)
    }

    fn decode(&self, bytes: &[u8], value_count: usize) -> io::Result<Vec<u32>> {
        if bytes.len() % 4 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Byte slice length must be a multiple of 4",
            ));
        }
        let mut output = Vec::new();
        for chunk in bytes.chunks_exact(4) {
            if output.len() == value_count {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Extra Simple9 words after the expected values",
                ));
            }
            let word = u32::from_le_bytes(chunk.try_into().unwrap());
            let selector = (word >> 28) as usize;
            let &(capacity, bits) = MODES.get(selector).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Invalid Simple9 selector")
            })?;
            // Ignore padding slots in the last word using the count from the block header.
            let count = capacity.min(value_count - output.len());
            let mask = (1_u32 << bits) - 1;

            for i in 0..count {
                output.push((word >> (i as u32 * bits)) & mask);
            }
        }
        if output.len() != value_count {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Not enough Simple9 values",
            ));
        }
        Ok(output)
    }
}
