use crate::app_loader::PackageImage;
use crate::content::ArmProfile;
use crate::error::{Result, SimulatorError};

pub const SYSTEM_RAM_BASE: u32 = 0x1000_0000;
pub const SYSTEM_RAM_SIZE: usize = 0x0400_0000;
pub const STACK_BASE: u32 = 0x1ff0_0000;
pub const STACK_SIZE: usize = 0x0010_0000;
pub const RETAIL_HEAP_BASE: u32 = 0x2100_0000;
pub const HOMEBREW_HEAP_BASE: u32 = 0x0900_0000;
pub const HEAP_SIZE: usize = 0x0200_0000;
pub const FRAMEBUFFER_BASE: u32 = 0x8000_0000;
pub const FRAMEBUFFER_SIZE: usize = 0x0080_0000;
pub const LEGACY_LOW_MEMORY_SIZE: usize = 0x0001_0000;
pub const DYNAMIC_THUNK_BASE: u32 = STACK_BASE + 0x1000;
pub const EXIT_ADDRESS: u32 = STACK_BASE + STACK_SIZE as u32 - 4;
const LEGACY_MMIO_BASE: u32 = 0x0400_0000;
const LEGACY_MMIO_SIZE: usize = 0x0010_0000;
const LEGACY_AUDIO_MMIO_BASE: u32 = 0x08a0_0000;
const LEGACY_AUDIO_MMIO_SIZE: usize = 0x0001_0000;
const LEGACY_SYSTEM_MMIO_BASE: u32 = 0x0930_0000;
const LEGACY_SYSTEM_MMIO_SIZE: usize = 0x0001_0000;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct A330Memory {
    profile: ArmProfile,
    system_ram: Vec<u8>,
    stack: Vec<u8>,
    heap: Vec<u8>,
    framebuffer: Vec<u8>,
    low_memory: Vec<u8>,
    legacy_mmio: Vec<u8>,
    legacy_audio_mmio: Vec<u8>,
    legacy_system_mmio: Vec<u8>,
}

impl A330Memory {
    pub fn from_package(package: &PackageImage) -> Result<Self> {
        let profile = package.arm_profile().ok_or_else(|| {
            SimulatorError::InvalidAppFormat("unsupported ARM memory profile".into())
        })?;
        let mut memory = Self {
            profile,
            system_ram: vec![0; SYSTEM_RAM_SIZE],
            stack: vec![0; STACK_SIZE],
            heap: vec![0; HEAP_SIZE],
            framebuffer: vec![0; FRAMEBUFFER_SIZE],
            low_memory: vec![0; LEGACY_LOW_MEMORY_SIZE],
            legacy_mmio: vec![0; LEGACY_MMIO_SIZE],
            legacy_audio_mmio: vec![0; LEGACY_AUDIO_MMIO_SIZE],
            legacy_system_mmio: vec![0; LEGACY_SYSTEM_MMIO_SIZE],
        };
        let program_end = package
            .load_base()
            .checked_add(package.program_size())
            .ok_or_else(|| SimulatorError::InvalidAppFormat("ARM program range overflow".into()))?;
        let system_end = SYSTEM_RAM_BASE + SYSTEM_RAM_SIZE as u32;
        if package.load_base() < SYSTEM_RAM_BASE || program_end > system_end {
            return Err(SimulatorError::InvalidAppFormat(format!(
                "ARM program range {:#010x}..{program_end:#010x} is outside system RAM",
                package.load_base()
            )));
        }
        memory.write_bytes(package.load_base(), package.executable())?;
        for (index, import) in package.imports.iter().enumerate() {
            if index > 0x00ff_ffff {
                return Err(SimulatorError::InvalidAppFormat(
                    "too many ARM imports".into(),
                ));
            }
            if import.address & 3 != 0 {
                return Err(SimulatorError::InvalidAppFormat(format!(
                    "unaligned ARM import {} at {:#010x}",
                    import.name, import.address
                )));
            }
            let stub_size = if profile == ArmProfile::Homebrew {
                4
            } else {
                8
            };
            let stub_end = import.address.checked_add(stub_size).ok_or_else(|| {
                SimulatorError::InvalidAppFormat("ARM import range overflow".into())
            })?;
            if import.address < package.load_base() || stub_end > program_end {
                return Err(SimulatorError::InvalidAppFormat(format!(
                    "ARM import {} lies outside the program image",
                    import.name
                )));
            }
            memory.write32(import.address, 0xef00_0000 | index as u32)?;
            if stub_size == 8 {
                memory.write32(import.address + 4, 0xe12f_ff1e)?;
            }
        }
        Ok(memory)
    }

    pub const fn profile(&self) -> ArmProfile {
        self.profile
    }
    pub const fn heap_base(&self) -> u32 {
        match self.profile {
            ArmProfile::Retail => RETAIL_HEAP_BASE,
            ArmProfile::Homebrew => HOMEBREW_HEAP_BASE,
        }
    }
    pub fn system_ram(&self) -> &[u8] {
        &self.system_ram
    }
    pub fn system_ram_mut(&mut self) -> &mut [u8] {
        &mut self.system_ram
    }
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }
    pub fn framebuffer_mut(&mut self) -> &mut [u8] {
        &mut self.framebuffer
    }

    pub(crate) fn is_cheat_writable_range(&self, address: u32, size: usize) -> bool {
        region_range(address, size, SYSTEM_RAM_BASE, self.system_ram.len()).is_some()
            || region_range(address, size, STACK_BASE, self.stack.len()).is_some()
            || region_range(address, size, self.heap_base(), self.heap.len()).is_some()
            || region_range(address, size, FRAMEBUFFER_BASE, self.framebuffer.len()).is_some()
    }

    pub(crate) fn snapshot_layout_is_valid(&self, profile: ArmProfile) -> bool {
        self.profile == profile
            && self.system_ram.len() == SYSTEM_RAM_SIZE
            && self.stack.len() == STACK_SIZE
            && self.heap.len() == HEAP_SIZE
            && self.framebuffer.len() == FRAMEBUFFER_SIZE
            && self.low_memory.len() == LEGACY_LOW_MEMORY_SIZE
            && self.legacy_mmio.len() == LEGACY_MMIO_SIZE
            && self.legacy_audio_mmio.len() == LEGACY_AUDIO_MMIO_SIZE
            && self.legacy_system_mmio.len() == LEGACY_SYSTEM_MMIO_SIZE
    }

    pub fn read8(&self, address: u32) -> Result<u8> {
        Ok(self.read_bytes(address, 1)?[0])
    }
    pub fn read16(&self, address: u32) -> Result<u16> {
        Ok(u16::from_le_bytes(
            self.read_bytes(address, 2)?.try_into().unwrap(),
        ))
    }
    pub fn read32(&self, address: u32) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.read_bytes(address, 4)?.try_into().unwrap(),
        ))
    }
    pub fn write8(&mut self, address: u32, value: u8) -> Result<()> {
        self.write_bytes(address, &[value])
    }
    pub fn write16(&mut self, address: u32, value: u16) -> Result<()> {
        self.write_bytes(address, &value.to_le_bytes())
    }
    pub fn write32(&mut self, address: u32, value: u32) -> Result<()> {
        self.write_bytes(address, &value.to_le_bytes())
    }

    pub fn read_bytes(&self, address: u32, size: usize) -> Result<&[u8]> {
        if let Some(range) = region_range(address, size, 0, self.low_memory.len()) {
            return Ok(&self.low_memory[range]);
        }
        if let Some(range) = region_range(address, size, SYSTEM_RAM_BASE, self.system_ram.len()) {
            return Ok(&self.system_ram[range]);
        }
        if let Some(range) = region_range(address, size, STACK_BASE, self.stack.len()) {
            return Ok(&self.stack[range]);
        }
        if let Some(range) = region_range(address, size, self.heap_base(), self.heap.len()) {
            return Ok(&self.heap[range]);
        }
        if let Some(range) = region_range(address, size, FRAMEBUFFER_BASE, self.framebuffer.len()) {
            return Ok(&self.framebuffer[range]);
        }
        if let Some(range) = region_range(address, size, LEGACY_MMIO_BASE, self.legacy_mmio.len()) {
            return Ok(&self.legacy_mmio[range]);
        }
        if let Some(range) = region_range(
            address,
            size,
            LEGACY_AUDIO_MMIO_BASE,
            self.legacy_audio_mmio.len(),
        ) {
            return Ok(&self.legacy_audio_mmio[range]);
        }
        if let Some(range) = region_range(
            address,
            size,
            LEGACY_SYSTEM_MMIO_BASE,
            self.legacy_system_mmio.len(),
        ) {
            return Ok(&self.legacy_system_mmio[range]);
        }
        Err(memory_error(address, size))
    }

    pub fn write_bytes(&mut self, address: u32, data: &[u8]) -> Result<()> {
        if let Some(range) = region_range(address, data.len(), 0, self.low_memory.len()) {
            self.low_memory[range].copy_from_slice(data);
            return Ok(());
        }
        if let Some(range) =
            region_range(address, data.len(), SYSTEM_RAM_BASE, self.system_ram.len())
        {
            self.system_ram[range].copy_from_slice(data);
            return Ok(());
        }
        if let Some(range) = region_range(address, data.len(), STACK_BASE, self.stack.len()) {
            self.stack[range].copy_from_slice(data);
            return Ok(());
        }
        let heap_base = self.heap_base();
        if let Some(range) = region_range(address, data.len(), heap_base, self.heap.len()) {
            self.heap[range].copy_from_slice(data);
            return Ok(());
        }
        if let Some(range) = region_range(
            address,
            data.len(),
            FRAMEBUFFER_BASE,
            self.framebuffer.len(),
        ) {
            self.framebuffer[range].copy_from_slice(data);
            return Ok(());
        }
        if let Some(range) = region_range(
            address,
            data.len(),
            LEGACY_MMIO_BASE,
            self.legacy_mmio.len(),
        ) {
            self.legacy_mmio[range].copy_from_slice(data);
            return Ok(());
        }
        if let Some(range) = region_range(
            address,
            data.len(),
            LEGACY_AUDIO_MMIO_BASE,
            self.legacy_audio_mmio.len(),
        ) {
            self.legacy_audio_mmio[range].copy_from_slice(data);
            return Ok(());
        }
        if let Some(range) = region_range(
            address,
            data.len(),
            LEGACY_SYSTEM_MMIO_BASE,
            self.legacy_system_mmio.len(),
        ) {
            self.legacy_system_mmio[range].copy_from_slice(data);
            return Ok(());
        }
        Err(memory_error(address, data.len()))
    }
}

fn region_range(
    address: u32,
    size: usize,
    base: u32,
    length: usize,
) -> Option<std::ops::Range<usize>> {
    let offset = address.checked_sub(base)? as usize;
    let end = offset.checked_add(size)?;
    (end <= length).then_some(offset..end)
}

fn memory_error(address: u32, size: usize) -> SimulatorError {
    SimulatorError::MemoryError {
        addr: address,
        message: format!("A330 access of {size} bytes is outside mapped memory"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_loader::{ChunkHeader, RawdHeader, SymbolEntry};
    use crate::content::ContentFormat;

    fn package(profile: ArmProfile) -> PackageImage {
        let origin = match profile {
            ArmProfile::Retail => ArmProfile::RETAIL_ORIGIN,
            ArmProfile::Homebrew => ArmProfile::HOMEBREW_ORIGIN,
        };
        let mut data = vec![0; 0xa0];
        for (index, byte) in data[0x80..].iter_mut().enumerate() {
            *byte = index as u8;
        }
        PackageImage {
            format: ContentFormat::Cc,
            data,
            impt: ChunkHeader::default(),
            expt: ChunkHeader::default(),
            rawd: RawdHeader {
                base: ChunkHeader {
                    ident: *b"RAWD",
                    chunk_type: 0,
                    offset: 0x80,
                    size: 0x20,
                },
                entry: origin,
                origin,
                program_size: 0x100,
            },
            has_erpt: false,
            erpt: ChunkHeader::default(),
            imports: vec![SymbolEntry {
                string_offset: 0,
                unknown0: 0,
                unknown1: 0,
                address: origin + 0x10,
                name: "test_import".into(),
            }],
            exports: Vec::new(),
            resources: Vec::new(),
        }
    }

    #[test]
    fn loads_program_zeros_bss_and_patches_retail_import() {
        let package = package(ArmProfile::Retail);
        let memory = A330Memory::from_package(&package).unwrap();
        assert_eq!(memory.read32(package.load_base()).unwrap(), 0x0302_0100);
        assert_eq!(
            memory.read32(package.load_base() + 0x10).unwrap(),
            0xef00_0000
        );
        assert_eq!(
            memory.read32(package.load_base() + 0x14).unwrap(),
            0xe12f_ff1e
        );
        assert_eq!(memory.read32(package.load_base() + 0x40).unwrap(), 0);
        assert_eq!(memory.heap_base(), RETAIL_HEAP_BASE);
    }

    #[test]
    fn homebrew_import_uses_single_instruction_layout() {
        let package = package(ArmProfile::Homebrew);
        let memory = A330Memory::from_package(&package).unwrap();
        assert_eq!(
            memory.read32(package.load_base() + 0x10).unwrap(),
            0xef00_0000
        );
        assert_eq!(
            memory.read32(package.load_base() + 0x14).unwrap(),
            0x1716_1514
        );
        assert_eq!(memory.heap_base(), HOMEBREW_HEAP_BASE);
    }

    #[test]
    fn mapped_regions_are_little_endian_and_bounds_checked() {
        let mut memory = A330Memory::from_package(&package(ArmProfile::Retail)).unwrap();
        memory.write32(STACK_BASE, 0x4433_2211).unwrap();
        assert_eq!(memory.read16(STACK_BASE + 1).unwrap(), 0x3322);
        memory.write8(FRAMEBUFFER_BASE, 0xaa).unwrap();
        assert_eq!(memory.framebuffer()[0], 0xaa);
        assert!(matches!(
            memory.write32(STACK_BASE + STACK_SIZE as u32 - 2, 0),
            Err(SimulatorError::MemoryError { .. })
        ));
        assert!(matches!(
            memory.read8(0x5000_0000),
            Err(SimulatorError::MemoryError { .. })
        ));
    }

    #[test]
    fn rejects_import_patch_outside_program() {
        let mut package = package(ArmProfile::Retail);
        package.imports[0].address = package.load_base() + package.program_size();
        assert!(matches!(
            A330Memory::from_package(&package),
            Err(SimulatorError::InvalidAppFormat(_))
        ));
    }
}
