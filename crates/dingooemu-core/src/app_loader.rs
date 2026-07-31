use crate::error::{Result, SimulatorError};
use std::path::Path;

/// Magic bytes for .app format chunks
#[allow(dead_code)]
const MAGIC_CCDL: &[u8; 4] = b"CCDL";
#[allow(dead_code)]
const MAGIC_IMPT: &[u8; 4] = b"IMPT";
#[allow(dead_code)]
const MAGIC_EXPT: &[u8; 4] = b"EXPT";
#[allow(dead_code)]
const MAGIC_RAWD: &[u8; 4] = b"RAWD";
#[allow(dead_code)]
const MAGIC_ERPT: &[u8; 4] = b"ERPT";

/// Fixed offsets for chunk descriptors
#[allow(dead_code)]
const CCDL_OFFSET: u32 = 0;
const IMPT_OFFSET: u32 = 0x20;
const EXPT_OFFSET: u32 = 0x40;
const RAWD_OFFSET: u32 = 0x60;
const ERPT_OFFSET: u32 = 0x80;

/// Minimum valid file size
const MIN_FILE_SIZE: usize = 0x80; // 128 bytes

/// Chunk descriptor (16 bytes)
#[derive(Debug, Clone, Default)]
pub struct ChunkHeader {
    /// 4-byte ASCII identifier
    pub ident: [u8; 4],
    /// Type field
    pub chunk_type: u32,
    /// Absolute file offset of chunk payload
    pub offset: u32,
    /// Byte size of chunk payload
    pub size: u32,
}

/// RAWD header (extended 32 bytes)
#[derive(Debug, Clone, Default)]
pub struct RawdHeader {
    /// Base chunk header
    pub base: ChunkHeader,
    /// Entry point address
    pub entry: u32,
    /// Load base address (origin)
    pub origin: u32,
    /// Total program size (>= rawd.size)
    pub program_size: u32,
}

/// Symbol table entry (16 bytes)
#[derive(Debug, Clone)]
pub struct SymbolEntry {
    /// Offset into string table
    pub string_offset: u32,
    /// Unknown field
    pub unknown0: u32,
    /// Unknown field
    pub unknown1: u32,
    /// Address/offset associated with this symbol
    pub address: u32,
    /// Resolved symbol name
    pub name: String,
}

/// Resource table entry
#[derive(Debug, Clone)]
pub struct ResourceEntry {
    /// Resource kind
    pub kind: ResourceKind,
    /// Resource name
    pub name: String,
    /// Absolute file offset
    pub offset: u32,
    /// Byte size
    pub size: u32,
    /// XOR key (0x40 for ERPT, 0 for others)
    pub xor_key: u8,
}

/// Resource table kind
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResourceKind {
    /// ERPT (XOR 0x40 encoded)
    Erpt,
    /// Packed (short-name format)
    Packed,
    /// Packed64 (long-path format)
    Packed64,
}

/// Complete parsed .app file structure
#[derive(Debug, Clone)]
pub struct AppImage {
    /// Raw file data
    pub data: Vec<u8>,
    /// Import table descriptor
    pub impt: ChunkHeader,
    /// Export table descriptor
    pub expt: ChunkHeader,
    /// RAWD header
    pub rawd: RawdHeader,
    /// Whether ERPT chunk exists
    pub has_erpt: bool,
    /// ERPT descriptor (if present)
    pub erpt: ChunkHeader,
    /// Parsed import symbols
    pub imports: Vec<SymbolEntry>,
    /// Parsed export symbols
    pub exports: Vec<SymbolEntry>,
    /// All detected resources
    pub resources: Vec<ResourceEntry>,
}

impl AppImage {
    /// Load and parse an .app file from disk
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = std::fs::read(path.as_ref())?;
        Self::parse(&data)
    }

    /// Parse .app data from a byte slice
    pub fn parse(data: &[u8]) -> Result<Self> {
        // Validate minimum size
        if data.len() < MIN_FILE_SIZE {
            return Err(SimulatorError::InvalidAppFormat(format!(
                "File too small: {} bytes (minimum {})",
                data.len(),
                MIN_FILE_SIZE
            )));
        }

        // Validate CCDL magic at offset 0
        if data[0..4] != *MAGIC_CCDL {
            return Err(SimulatorError::InvalidAppFormat(
                "Invalid CCDL magic".to_string(),
            ));
        }

        // Parse chunk descriptors at fixed offsets
        let impt = read_chunk_header(data, IMPT_OFFSET as usize)?;
        let expt = read_chunk_header(data, EXPT_OFFSET as usize)?;
        let mut rawd = read_rawd_header(data, RAWD_OFFSET as usize)?;

        // Validate RAWD
        if rawd.entry == 0 {
            return Err(SimulatorError::InvalidAppFormat(
                "RAW entry point is zero".to_string(),
            ));
        }
        let rawd_end = rawd
            .base
            .offset
            .checked_add(rawd.base.size)
            .ok_or_else(|| {
                SimulatorError::InvalidAppFormat("RAW payload range overflow".to_string())
            })?;
        if rawd_end as usize > data.len() {
            return Err(SimulatorError::InvalidAppFormat(
                "RAW payload out of bounds".to_string(),
            ));
        }
        if rawd.program_size < rawd.base.size {
            log::warn!(
                "RAW program_size {} is smaller than payload {}; using payload size",
                rawd.program_size,
                rawd.base.size
            );
            rawd.program_size = rawd.base.size;
        }

        // Check for ERPT chunk
        let (has_erpt, erpt) = if data.len() >= ERPT_OFFSET as usize + 16 {
            let chunk = read_chunk_header(data, ERPT_OFFSET as usize)?;
            if chunk.ident == *MAGIC_ERPT {
                (true, chunk)
            } else {
                (false, ChunkHeader::default())
            }
        } else {
            (false, ChunkHeader::default())
        };

        // Parse symbol tables
        let imports = if impt.size > 0 {
            parse_symbol_table(data, impt.offset as usize, impt.size as usize)?
        } else {
            Vec::new()
        };

        let exports = if expt.size > 0 {
            parse_symbol_table(data, expt.offset as usize, expt.size as usize)?
        } else {
            Vec::new()
        };

        // Parse resources
        let resources = parse_resources(data, &rawd, has_erpt, &erpt)?;

        log::info!(
            "Parsed .app: entry={:#010x}, base={:#010x}, size={}, imports={}, exports={}, resources={}",
            rawd.entry,
            rawd.origin,
            rawd.program_size,
            imports.len(),
            exports.len(),
            resources.len()
        );

        Ok(Self {
            data: data.to_vec(),
            impt,
            expt,
            rawd,
            has_erpt,
            erpt,
            imports,
            exports,
            resources,
        })
    }

    /// Get the executable data (RAWD payload)
    pub fn executable(&self) -> &[u8] {
        let start = self.rawd.base.offset as usize;
        let end = start + self.rawd.base.size as usize;
        &self.data[start..end]
    }

    /// Get the entry point address
    pub fn entry_point(&self) -> u32 {
        self.rawd.entry
    }

    /// Get the load base address
    pub fn load_base(&self) -> u32 {
        self.rawd.origin
    }

    /// Get program size
    pub fn program_size(&self) -> u32 {
        self.rawd.program_size
    }

    /// Find a resource by name, accepting path separator and basename variants.
    pub fn find_resource(&self, name: &str) -> Option<&ResourceEntry> {
        let requested = normalize_resource_name(name);
        self.resources
            .iter()
            .find(|r| normalize_resource_name(&r.name) == requested)
            .or_else(|| {
                let requested_base = resource_basename(&requested);
                self.resources.iter().find(|r| {
                    resource_basename(&normalize_resource_name(&r.name)) == requested_base
                })
            })
    }

    /// Get resource data (decoded)
    pub fn get_resource_data(&self, resource: &ResourceEntry) -> Vec<u8> {
        let start = resource.offset as usize;
        let end = start + resource.size as usize;
        let mut data = self.data[start..end].to_vec();

        // Apply XOR decoding if needed
        if resource.xor_key != 0 {
            for b in &mut data {
                *b ^= resource.xor_key;
            }
        }

        data
    }
}

/// Read a 16-byte chunk header
fn read_chunk_header(data: &[u8], offset: usize) -> Result<ChunkHeader> {
    if data.len() < offset + 16 {
        return Err(SimulatorError::InvalidAppFormat(format!(
            "Chunk header at {:#x} out of bounds",
            offset
        )));
    }

    let ident = [
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ];
    let chunk_type = u32::from_le_bytes([
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ]);
    let chunk_offset = u32::from_le_bytes([
        data[offset + 8],
        data[offset + 9],
        data[offset + 10],
        data[offset + 11],
    ]);
    let size = u32::from_le_bytes([
        data[offset + 12],
        data[offset + 13],
        data[offset + 14],
        data[offset + 15],
    ]);

    Ok(ChunkHeader {
        ident,
        chunk_type,
        offset: chunk_offset,
        size,
    })
}

/// Read the RAWD header (extended 32 bytes)
fn read_rawd_header(data: &[u8], offset: usize) -> Result<RawdHeader> {
    if data.len() < offset + 32 {
        return Err(SimulatorError::InvalidAppFormat(
            "RAW header out of bounds".to_string(),
        ));
    }

    let base = read_chunk_header(data, offset)?;

    let entry = u32::from_le_bytes([
        data[offset + 20],
        data[offset + 21],
        data[offset + 22],
        data[offset + 23],
    ]);
    let origin = u32::from_le_bytes([
        data[offset + 24],
        data[offset + 25],
        data[offset + 26],
        data[offset + 27],
    ]);
    let program_size = u32::from_le_bytes([
        data[offset + 28],
        data[offset + 29],
        data[offset + 30],
        data[offset + 31],
    ]);

    Ok(RawdHeader {
        base,
        entry,
        origin,
        program_size,
    })
}

/// Parse a symbol table (IMPT or EXPT)
fn parse_symbol_table(data: &[u8], offset: usize, size: usize) -> Result<Vec<SymbolEntry>> {
    if size < 16 {
        return Ok(Vec::new());
    }

    // Read count (first 4 bytes)
    let count = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;

    // Sanity check
    if count > 4096 {
        log::warn!("Symbol table count {} too large, skipping", count);
        return Ok(Vec::new());
    }

    // String table starts after 16-byte header + count * 16-byte entries
    let entries_start = offset + 16;
    let strings_start = entries_start + count * 16;

    if strings_start > offset + size {
        return Err(SimulatorError::InvalidAppFormat(
            "Symbol table truncated".to_string(),
        ));
    }

    let mut symbols = Vec::with_capacity(count);

    for i in 0..count {
        let entry_offset = entries_start + i * 16;

        let string_offset = u32::from_le_bytes([
            data[entry_offset],
            data[entry_offset + 1],
            data[entry_offset + 2],
            data[entry_offset + 3],
        ]);
        let unknown0 = u32::from_le_bytes([
            data[entry_offset + 4],
            data[entry_offset + 5],
            data[entry_offset + 6],
            data[entry_offset + 7],
        ]);
        let unknown1 = u32::from_le_bytes([
            data[entry_offset + 8],
            data[entry_offset + 9],
            data[entry_offset + 10],
            data[entry_offset + 11],
        ]);
        let address = u32::from_le_bytes([
            data[entry_offset + 12],
            data[entry_offset + 13],
            data[entry_offset + 14],
            data[entry_offset + 15],
        ]);

        // Resolve name from string table
        let name = read_cstring(data, strings_start + string_offset as usize, 256);

        symbols.push(SymbolEntry {
            string_offset,
            unknown0,
            unknown1,
            address,
            name,
        });
    }

    Ok(symbols)
}

/// Parse resource tables (ERPT, packed, packed64)
fn parse_resources(
    data: &[u8],
    rawd: &RawdHeader,
    has_erpt: bool,
    erpt: &ChunkHeader,
) -> Result<Vec<ResourceEntry>> {
    let mut resources = Vec::new();

    // Try ERPT first
    if has_erpt && erpt.size > 0 {
        resources = parse_erpt_resources(data, erpt)?;
        if !resources.is_empty() {
            return Ok(resources);
        }
    }

    // Try packed resources after RAWD payload, scanning aligned package tables.
    let rawd_end = rawd.base.offset + rawd.base.size;
    if rawd_end < data.len() as u32 {
        resources = parse_packed_resources(data, rawd_end as usize);
        if !resources.is_empty() {
            return Ok(resources);
        }
    }

    // Try packed64 resources
    if rawd_end < data.len() as u32 {
        resources = parse_packed64_resources(data, rawd_end as usize);
    }

    Ok(resources)
}

/// Parse ERPT resources (XOR 0x40 encoded)
fn parse_erpt_resources(data: &[u8], erpt: &ChunkHeader) -> Result<Vec<ResourceEntry>> {
    const RECORD_SIZE: usize = 0x1FC;
    const NAME_SIZE: usize = 0x1F4;

    let offset = erpt.offset as usize;
    let size = erpt.size as usize;

    if size < 4 {
        return Ok(Vec::new());
    }
    if offset > data.len().saturating_sub(4) {
        return Ok(Vec::new());
    }
    let chunk_end = offset.saturating_add(size).min(data.len());
    let declared_count = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;
    let count = declared_count.min((size - 4) / RECORD_SIZE);

    let mut resources = Vec::with_capacity(count);

    for i in 0..count {
        let record_offset = offset + 4 + i * RECORD_SIZE;

        if record_offset + RECORD_SIZE > chunk_end {
            break;
        }

        // Read name (null-terminated)
        let name = read_cstring(data, record_offset, NAME_SIZE);

        // Read size and relative offset
        let res_size = u32::from_le_bytes([
            data[record_offset + NAME_SIZE],
            data[record_offset + NAME_SIZE + 1],
            data[record_offset + NAME_SIZE + 2],
            data[record_offset + NAME_SIZE + 3],
        ]);
        let rel_offset = u32::from_le_bytes([
            data[record_offset + NAME_SIZE + 4],
            data[record_offset + NAME_SIZE + 5],
            data[record_offset + NAME_SIZE + 6],
            data[record_offset + NAME_SIZE + 7],
        ]);

        // Absolute offset = ERPT chunk offset + relative offset
        let abs_offset = erpt.offset + rel_offset;

        resources.push(ResourceEntry {
            kind: ResourceKind::Erpt,
            name,
            offset: abs_offset,
            size: res_size,
            xor_key: 0x40,
        });
    }

    Ok(resources)
}

/// Parse packed resources (short-name format, 36-byte records)
fn parse_packed_resources(data: &[u8], start: usize) -> Vec<ResourceEntry> {
    const MAX_TABLES: usize = 8;
    const SCAN_ALIGNMENT: usize = 0x1000;

    let mut table_starts = Vec::new();

    if is_valid_packed_table(data, start) {
        table_starts.push(start);
    }

    let mut scan = align_up(start, SCAN_ALIGNMENT);
    while scan + 2 < data.len() && table_starts.len() < MAX_TABLES {
        if scan != start && is_valid_packed_table(data, scan) {
            table_starts.push(scan);
        }
        scan = scan.saturating_add(SCAN_ALIGNMENT);
    }

    let mut resources = Vec::new();
    for (index, &table_start) in table_starts.iter().enumerate() {
        let package_end = table_starts.get(index + 1).copied().unwrap_or(data.len());
        parse_packed_table(data, table_start, package_end, &mut resources);
    }

    resources
}

fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

fn is_valid_packed_table(data: &[u8], start: usize) -> bool {
    const RECORD_SIZE: usize = 36;
    const NAME_SIZE: usize = 32;
    const MIN_VALID_SAMPLE: usize = 4;

    if data.len() < start + 2 {
        return false;
    }

    let count = u16::from_le_bytes([data[start], data[start + 1]]) as usize;
    if count == 0 || count > 1024 {
        return false;
    }

    let table_size = 2 + count * RECORD_SIZE;
    if start + table_size > data.len() {
        return false;
    }

    let sample_count = count.min(32);
    let mut valid = 0usize;
    let mut last_offset = table_size as u32;

    for i in 0..sample_count {
        let record_offset = start + 2 + i * RECORD_SIZE;
        let name = read_cstring(data, record_offset, NAME_SIZE);
        let rel_offset = read_u32_at(data, record_offset + NAME_SIZE);

        if !name.is_empty()
            && name.contains('.')
            && is_printable_ascii(&name)
            && rel_offset >= table_size as u32
            && start + (rel_offset as usize) < data.len()
            && rel_offset >= last_offset
        {
            valid += 1;
            last_offset = rel_offset;
        }
    }

    valid >= MIN_VALID_SAMPLE && valid * 2 >= sample_count
}

fn parse_packed_table(
    data: &[u8],
    start: usize,
    package_end: usize,
    resources: &mut Vec<ResourceEntry>,
) {
    const RECORD_SIZE: usize = 36;
    const NAME_SIZE: usize = 32;

    let count = u16::from_le_bytes([data[start], data[start + 1]]) as usize;
    let table_size = 2 + count * RECORD_SIZE;
    let mut entries = Vec::new();

    for i in 0..count {
        let record_offset = start + 2 + i * RECORD_SIZE;
        let name = read_cstring(data, record_offset, NAME_SIZE);
        let rel_offset = read_u32_at(data, record_offset + NAME_SIZE) as usize;

        if name.is_empty()
            || !name.contains('.')
            || !is_printable_ascii(&name)
            || rel_offset < table_size
            || start + rel_offset >= package_end
        {
            continue;
        }

        entries.push((start + rel_offset, name));
    }

    entries.sort_by_key(|(offset, _)| *offset);
    entries.dedup_by(|a, b| {
        a.0 == b.0 && normalize_resource_name(&a.1) == normalize_resource_name(&b.1)
    });

    for i in 0..entries.len() {
        let (offset, name) = &entries[i];
        let next_offset = entries
            .iter()
            .skip(i + 1)
            .map(|(next, _)| *next)
            .find(|next| next > offset)
            .unwrap_or(package_end);

        if next_offset > *offset {
            resources.push(ResourceEntry {
                kind: ResourceKind::Packed,
                name: name.clone(),
                offset: *offset as u32,
                size: (next_offset - *offset) as u32,
                xor_key: 0,
            });
        }
    }
}

fn read_u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}
/// Parse packed64 resources (long-path format, 68-byte records)
fn parse_packed64_resources(data: &[u8], start: usize) -> Vec<ResourceEntry> {
    const RECORD_SIZE: usize = 0x44;
    const NAME_SIZE: usize = 0x40;

    let mut resources = Vec::new();
    let mut offset = start;

    // Read records until invalid entry
    let mut prev_offset = 0u32;
    let mut first_stored_offset = None;

    loop {
        if offset + RECORD_SIZE > data.len() {
            break;
        }

        let stored_offset = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);

        // Read name (null-terminated)
        let name = read_cstring(data, offset + 4, NAME_SIZE);

        // Validate: name should start with ".\" and contain a dot
        if !name.starts_with(".\\") || !name.contains('.') || !is_printable_ascii(&name) {
            break;
        }

        // Validate offset is monotonically non-decreasing
        if stored_offset < prev_offset && first_stored_offset.is_some() {
            break;
        }

        if first_stored_offset.is_none() {
            first_stored_offset = Some(stored_offset);
        }

        prev_offset = stored_offset;
        offset += RECORD_SIZE;
    }

    // Calculate data bias (first stored offset - table end)
    if let Some(first_offset) = first_stored_offset {
        let table_end = offset as u32;
        let data_bias = first_offset - table_end;

        // Re-parse with correct offsets
        offset = start;
        let mut prev = 0u32;
        let mut stored_offsets = Vec::new();

        loop {
            if offset + RECORD_SIZE > data.len() {
                break;
            }

            let stored = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            let name = read_cstring(data, offset + 4, NAME_SIZE);

            if !name.starts_with(".\\") || !name.contains('.') || !is_printable_ascii(&name) {
                break;
            }

            if stored < prev && !stored_offsets.is_empty() {
                break;
            }

            stored_offsets.push((stored, name));
            prev = stored;
            offset += RECORD_SIZE;
        }

        // Convert to ResourceEntry
        for i in 0..stored_offsets.len() {
            let (stored, name) = &stored_offsets[i];
            let abs_offset = stored - data_bias;

            let size = if i + 1 < stored_offsets.len() {
                stored_offsets[i + 1].0 - data_bias - abs_offset
            } else {
                // Approximate size
                (data.len() as u32) - abs_offset
            };

            resources.push(ResourceEntry {
                kind: ResourceKind::Packed64,
                name: name.clone(),
                offset: abs_offset,
                size,
                xor_key: 0,
            });
        }
    }

    resources
}

/// Read a null-terminated C string
fn read_cstring(data: &[u8], offset: usize, max_len: usize) -> String {
    let mut s = String::new();
    for i in 0..max_len {
        if offset + i >= data.len() {
            break;
        }
        let b = data[offset + i];
        if b == 0 {
            break;
        }
        s.push(b as char);
    }
    s
}

fn normalize_resource_name(name: &str) -> String {
    let mut s = name.replace('/', "\\").to_ascii_lowercase();
    while let Some(stripped) = s.strip_prefix(".\\") {
        s = stripped.to_string();
    }
    while let Some(stripped) = s.strip_prefix('\\') {
        s = stripped.to_string();
    }
    s
}

fn resource_basename(name: &str) -> &str {
    name.rsplit(['\\', '/', ':']).next().unwrap_or(name)
}
/// Check if a string is printable ASCII
fn is_printable_ascii(s: &str) -> bool {
    s.bytes()
        .all(|b| (0x20..0x7F).contains(&b) || b == b'\t' || b == b'\n' || b == b'\r')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_chunk_header() {
        let mut data = vec![0u8; 64];
        // Write IMPT header at offset 32
        data[32..36].copy_from_slice(b"IMPT");
        data[36..40].copy_from_slice(&1u32.to_le_bytes());
        data[40..44].copy_from_slice(&0x1000u32.to_le_bytes());
        data[44..48].copy_from_slice(&0x200u32.to_le_bytes());

        let header = read_chunk_header(&data, 32).unwrap();
        assert_eq!(header.ident, *MAGIC_IMPT);
        assert_eq!(header.chunk_type, 1);
        assert_eq!(header.offset, 0x1000);
        assert_eq!(header.size, 0x200);
    }

    #[test]
    fn test_read_rawd_header() {
        let mut data = vec![0u8; 128];
        // Write RAWD header at offset 96
        data[96..100].copy_from_slice(b"RAWD");
        data[116..120].copy_from_slice(&0x8000_0000u32.to_le_bytes()); // entry
        data[120..124].copy_from_slice(&0x8000_0000u32.to_le_bytes()); // origin
        data[124..128].copy_from_slice(&0x1000u32.to_le_bytes()); // program_size

        let header = read_rawd_header(&data, 96).unwrap();
        assert_eq!(header.base.ident, *MAGIC_RAWD);
        assert_eq!(header.entry, 0x8000_0000);
        assert_eq!(header.origin, 0x8000_0000);
        assert_eq!(header.program_size, 0x1000);
    }

    #[test]
    fn test_parse_uses_raw_payload_size_when_program_size_is_smaller() {
        let mut data = vec![0u8; 256];
        data[0..4].copy_from_slice(b"CCDL");
        data[0x20..0x24].copy_from_slice(b"IMPT");
        data[0x40..0x44].copy_from_slice(b"EXPT");
        data[0x60..0x64].copy_from_slice(b"RAWD");
        data[0x68..0x6C].copy_from_slice(&0x80u32.to_le_bytes());
        data[0x6C..0x70].copy_from_slice(&0x20u32.to_le_bytes());
        data[0x74..0x78].copy_from_slice(&0x8000_0000u32.to_le_bytes());
        data[0x78..0x7C].copy_from_slice(&0x8000_0000u32.to_le_bytes());
        data[0x7C..0x80].copy_from_slice(&0x18u32.to_le_bytes());

        let app = AppImage::parse(&data).unwrap();

        assert_eq!(app.program_size(), 0x20);
        assert_eq!(app.executable().len(), 0x20);
    }

    #[test]
    fn test_read_cstring() {
        let data = b"Hello\0World\0";
        assert_eq!(read_cstring(data, 0, 100), "Hello");
        assert_eq!(read_cstring(data, 6, 100), "World");
    }

    #[test]
    fn test_is_printable_ascii() {
        assert!(is_printable_ascii("Hello"));
        assert!(is_printable_ascii("Test 123"));
        assert!(is_printable_ascii("Hello\nWorld")); // Newline is allowed
        assert!(is_printable_ascii("Hello\tWorld")); // Tab is allowed
        assert!(is_printable_ascii("Hello\r\nWorld")); // CR+LF is allowed
    }

    #[test]
    fn test_erpt_out_of_file_range_is_ignored() {
        let data = vec![0u8; 128];
        let erpt = ChunkHeader {
            offset: data.len() as u32,
            size: 64,
            ..ChunkHeader::default()
        };

        let resources = parse_erpt_resources(&data, &erpt).unwrap();

        assert!(resources.is_empty());
    }

    #[test]
    fn test_erpt_size_is_clamped_to_file() {
        const RECORD_SIZE: usize = 0x1FC;
        let mut data = vec![0u8; 4 + RECORD_SIZE];
        data[0..4].copy_from_slice(&1u32.to_le_bytes());
        data[4..14].copy_from_slice(b"asset.bin\0");
        let erpt = ChunkHeader {
            offset: 0,
            size: (data.len() + 128) as u32,
            ..ChunkHeader::default()
        };

        let resources = parse_erpt_resources(&data, &erpt).unwrap();

        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].name, "asset.bin");
    }
}
