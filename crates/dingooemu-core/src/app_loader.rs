use crate::error::{Result, SimulatorError};
use std::path::Path;

/// Magic bytes for .app format chunks
const MAGIC_CCDL: &[u8; 4] = b"CCDL";
#[allow(dead_code)]
const MAGIC_IMPT: &[u8; 4] = b"IMPT";
#[allow(dead_code)]
const MAGIC_EXPT: &[u8; 4] = b"EXPT";
const MAGIC_RAWD: &[u8; 4] = b"RAWD";
#[allow(dead_code)]
const MAGIC_ERPT: &[u8; 4] = b"ERPT";

/// Parsed .app file structure
pub struct AppFile {
    /// Raw executable data (RAWD)
    pub executable: Vec<u8>,
    /// Entry point address
    pub entry_point: u32,
    /// Load base address
    pub load_base: u32,
    /// Program size
    pub program_size: u32,
}

impl AppFile {
    /// Load and parse an .app file from disk
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = std::fs::read(path.as_ref())?;
        Self::parse(&data)
    }

    /// Parse .app data from a byte slice
    pub fn parse(data: &[u8]) -> Result<Self> {
        // Find CCDL chunk
        let ccdl_offset = find_chunk(data, MAGIC_CCDL)
            .ok_or_else(|| SimulatorError::InvalidAppFormat("CCDL chunk not found".to_string()))?;

        // Parse CCDL header (offset 0x10 contains entry point)
        if data.len() < ccdl_offset + 0x20 {
            return Err(SimulatorError::InvalidAppFormat(
                "CCDL chunk too small".to_string(),
            ));
        }
        let entry_point = u32::from_le_bytes([
            data[ccdl_offset + 0x10],
            data[ccdl_offset + 0x11],
            data[ccdl_offset + 0x12],
            data[ccdl_offset + 0x13],
        ]);

        // Find RAWD chunk
        let rawd_offset = find_chunk(data, MAGIC_RAWD)
            .ok_or_else(|| SimulatorError::InvalidAppFormat("RAWD chunk not found".to_string()))?;

        // Parse RAWD header
        if data.len() < rawd_offset + 0x10 {
            return Err(SimulatorError::InvalidAppFormat(
                "RAWD chunk too small".to_string(),
            ));
        }

        let load_base = u32::from_le_bytes([
            data[rawd_offset + 4],
            data[rawd_offset + 5],
            data[rawd_offset + 6],
            data[rawd_offset + 7],
        ]);

        let program_size = u32::from_le_bytes([
            data[rawd_offset + 8],
            data[rawd_offset + 9],
            data[rawd_offset + 10],
            data[rawd_offset + 11],
        ]);

        // Extract executable data (starts after RAWD header)
        let exec_start = rawd_offset + 0x10;
        let exec_end = exec_start + program_size as usize;

        if data.len() < exec_end {
            return Err(SimulatorError::InvalidAppFormat(format!(
                "RAWD data truncated: need {} bytes, have {}",
                program_size,
                data.len() - exec_start
            )));
        }

        let executable = data[exec_start..exec_end].to_vec();

        log::info!(
            "Loaded .app: entry={:#010x}, base={:#010x}, size={}",
            entry_point,
            load_base,
            program_size
        );

        Ok(Self {
            executable,
            entry_point,
            load_base,
            program_size,
        })
    }
}

/// Find a chunk by magic bytes in the data
fn find_chunk(data: &[u8], magic: &[u8; 4]) -> Option<usize> {
    data.windows(4).position(|w| w == magic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_chunk() {
        let mut data = vec![0u8; 0x100];
        data[0x20..0x24].copy_from_slice(MAGIC_CCDL);
        assert_eq!(find_chunk(&data, MAGIC_CCDL), Some(0x20));
        assert_eq!(find_chunk(&data, MAGIC_RAWD), None);
    }
}
