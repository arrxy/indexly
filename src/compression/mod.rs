//! Integer codecs used independently for document gaps and term frequencies.

use std::io;

mod simple9;
pub use simple9::Simple9;

/// Encodes independent blocks; the caller supplies the exact decoded value count.
pub trait IntegerCodec {
    /// Identifier persisted in the IDX1 header; readers must use the same codec.
    fn id(&self) -> u8;
    /// Encode one integer sequence without embedding its length.
    fn encode(&self, values: &[u32]) -> io::Result<Vec<u8>>;
    /// Decode exactly `value_count` integers, excluding final-word padding.
    fn decode(&self, bytes: &[u8], value_count: usize) -> io::Result<Vec<u32>>;
}
