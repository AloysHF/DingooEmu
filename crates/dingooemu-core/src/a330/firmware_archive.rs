use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

const HEADER_SIZE: usize = 64;
const MAGIC: &[u8; 8] = b"WADF0100";

#[derive(Clone)]
pub(crate) struct FirmwareArchive {
    data: Arc<[u8]>,
    entries: BTreeMap<String, Range<usize>>,
}

impl FirmwareArchive {
    pub(crate) fn discover(directory: &Path) -> Option<Self> {
        let path = std::fs::read_dir(directory)
            .ok()?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("hxf"))
            })?;
        match std::fs::read(&path) {
            Ok(data) => match Self::parse(data) {
                Ok(archive) => {
                    log::info!(
                        "Indexed A330 firmware archive {} with {} files",
                        path.display(),
                        archive.entries.len()
                    );
                    Some(archive)
                }
                Err(message) => {
                    log::warn!(
                        "Ignoring invalid A330 firmware archive {}: {message}",
                        path.display()
                    );
                    None
                }
            },
            Err(error) => {
                log::warn!(
                    "Unable to read A330 firmware archive {}: {error}",
                    path.display()
                );
                None
            }
        }
    }

    pub(crate) fn parse(data: Vec<u8>) -> std::result::Result<Self, String> {
        if data.len() < HEADER_SIZE || &data[..MAGIC.len()] != MAGIC {
            return Err("missing WADF0100 header".into());
        }
        let declared_size = read_u32(&data, 20)? as usize;
        if declared_size < HEADER_SIZE || declared_size > data.len() {
            return Err("declared archive size is outside the file".into());
        }

        let mut entries = BTreeMap::new();
        let mut cursor = HEADER_SIZE;
        loop {
            let name_length = read_u32(&data, cursor)? as usize;
            cursor = cursor
                .checked_add(4)
                .ok_or("firmware entry offset overflow")?;
            if name_length == 0 {
                break;
            }
            if name_length > 4096 {
                return Err("firmware entry name is too long".into());
            }
            let name_end = cursor
                .checked_add(name_length)
                .filter(|end| *end < declared_size)
                .ok_or("firmware entry name is truncated")?;
            let name = String::from_utf8_lossy(&data[cursor..name_end]);
            let size_offset = name_end
                .checked_add(1)
                .ok_or("firmware entry size offset overflow")?;
            let size = read_u32(&data, size_offset)? as usize;
            let data_start = size_offset
                .checked_add(4)
                .ok_or("firmware entry data offset overflow")?;
            let data_end = data_start
                .checked_add(size)
                .filter(|end| *end <= declared_size)
                .ok_or("firmware entry data is truncated")?;
            entries
                .entry(normalize_path(&name))
                .or_insert(data_start..data_end);
            cursor = data_end;
        }

        Ok(Self {
            data: Arc::from(data),
            entries,
        })
    }

    pub(crate) fn read(&self, name: &str) -> Option<Vec<u8>> {
        let range = self.entries.get(&normalize_path(name))?;
        Some(self.data[range.clone()].to_vec())
    }
}

fn read_u32(data: &[u8], offset: usize) -> std::result::Result<u32, String> {
    let bytes = data
        .get(offset..offset.saturating_add(4))
        .ok_or("firmware archive is truncated")?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn normalize_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let without_drive = normalized
        .get(1..3)
        .filter(|prefix| prefix.starts_with(':') && prefix.ends_with('/'))
        .map_or(normalized.as_str(), |_| &normalized[3..]);
    without_drive.trim_start_matches("./").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut data = vec![0; HEADER_SIZE];
        data[..MAGIC.len()].copy_from_slice(MAGIC);
        for (name, contents) in entries {
            data.extend_from_slice(&(name.len() as u32).to_le_bytes());
            data.extend_from_slice(name.as_bytes());
            data.push(b' ');
            data.extend_from_slice(&(contents.len() as u32).to_le_bytes());
            data.extend_from_slice(contents);
        }
        data.extend_from_slice(&0_u32.to_le_bytes());
        let size = data.len() as u32;
        data[20..24].copy_from_slice(&size.to_le_bytes());
        data
    }

    #[test]
    fn parses_files_and_resolves_guest_drive_paths() {
        let archive = FirmwareArchive::parse(archive(&[
            ("system\\nls\\c_936.nls", &[1, 2, 3]),
            ("SYSTEM\\FONT\\font.bmf", &[4, 5]),
        ]))
        .unwrap();
        assert_eq!(
            archive.read("z:\\system\\nls\\c_936.nls"),
            Some(vec![1, 2, 3])
        );
        assert_eq!(archive.read("a:/system/font/FONT.bmf"), Some(vec![4, 5]));
        assert_eq!(archive.read("z:\\missing.bin"), None);
    }

    #[test]
    fn rejects_truncated_entries() {
        let mut data = archive(&[("file.bin", &[1, 2, 3])]);
        data.truncate(data.len() - 3);
        assert!(FirmwareArchive::parse(data).is_err());
    }
}
