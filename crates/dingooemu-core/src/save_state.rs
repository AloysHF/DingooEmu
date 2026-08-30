use anyhow::{bail, Context, Result};
use bincode::Options;
use serde::de::DeserializeOwned;
use serde::Serialize;

const MAGIC: &[u8; 8] = b"DINGSTAT";
const VERSION: u32 = 3;
const HEADER_SIZE: usize = 32;
// A Dingoo snapshot always contains the 32 MiB guest RAM and may also contain
// resources that the guest deliberately keeps open. Real applications can
// therefore exceed the original 64 MiB ceiling even though their LZ4 payload
// still fits in the fixed libretro serialization buffer.
const MAX_DECODED_SIZE: usize = 128 * 1024 * 1024;

/// Fixed capacity required by the libretro serialization API.
pub const SERIALIZED_SIZE: usize = 48 * 1024 * 1024;

pub fn encode<T: Serialize>(value: &T, content_crc32: u32, output: &mut [u8]) -> Result<()> {
    if output.len() < SERIALIZED_SIZE {
        bail!(
            "save-state buffer is too small: got {}, need {}",
            output.len(),
            SERIALIZED_SIZE
        );
    }

    let decoded = codec()
        .serialize(value)
        .context("failed to encode save-state payload")?;
    if decoded.len() > MAX_DECODED_SIZE {
        bail!(
            "save-state decoded payload is {} bytes; limit is {} bytes",
            decoded.len(),
            MAX_DECODED_SIZE
        );
    }

    let payload = lz4_flex::compress(&decoded);
    if payload.len() > SERIALIZED_SIZE - HEADER_SIZE {
        bail!(
            "save-state compressed payload is {} bytes; fixed capacity is {} bytes",
            payload.len(),
            SERIALIZED_SIZE - HEADER_SIZE
        );
    }

    output.fill(0);
    output[..8].copy_from_slice(MAGIC);
    output[8..12].copy_from_slice(&VERSION.to_le_bytes());
    output[12..16].copy_from_slice(&content_crc32.to_le_bytes());
    output[16..20].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    output[20..24].copy_from_slice(&(decoded.len() as u32).to_le_bytes());
    output[24..28].copy_from_slice(&crc32fast::hash(&payload).to_le_bytes());
    output[HEADER_SIZE..HEADER_SIZE + payload.len()].copy_from_slice(&payload);
    Ok(())
}

pub fn decode<T: DeserializeOwned>(input: &[u8], expected_content_crc32: u32) -> Result<T> {
    if input.len() < HEADER_SIZE {
        bail!("save state is truncated");
    }
    if &input[..8] != MAGIC {
        bail!("invalid save-state signature");
    }

    let version = read_u32(input, 8);
    if version != VERSION {
        bail!("unsupported save-state version {version}");
    }
    if read_u32(input, 12) != expected_content_crc32 {
        bail!("save state belongs to different content data");
    }

    let payload_len = read_u32(input, 16) as usize;
    let decoded_len = read_u32(input, 20) as usize;
    if decoded_len > MAX_DECODED_SIZE {
        bail!(
            "save-state declares {} decoded bytes; limit is {} bytes",
            decoded_len,
            MAX_DECODED_SIZE
        );
    }
    let payload_end = HEADER_SIZE
        .checked_add(payload_len)
        .filter(|&end| end <= input.len() && end <= SERIALIZED_SIZE)
        .context("invalid save-state payload length")?;
    let payload = &input[HEADER_SIZE..payload_end];
    if crc32fast::hash(payload) != read_u32(input, 24) {
        bail!("save-state checksum mismatch");
    }

    let decoded =
        lz4_flex::decompress(payload, decoded_len).context("failed to decompress save state")?;
    codec()
        .with_limit(MAX_DECODED_SIZE as u64)
        .deserialize(&decoded)
        .context("failed to decode save state")
}

fn codec() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
}

fn read_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(input[offset..offset + 4].try_into().unwrap())
}
