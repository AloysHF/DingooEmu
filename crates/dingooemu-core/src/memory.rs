use crate::error::{Result, SimulatorError};

/// Dingoo A320 memory regions
const RAM_BASE: u32 = 0x0000_0000;
const RAM_SIZE: u32 = 32 * 1024 * 1024; // 32 MB

/// MIPS KSEG0 mask (strip top 3 bits for cached segment)
const KSEG0_MASK: u32 = 0x1FFF_FFFF;

/// Memory manager for the Dingoo A320
pub struct Memory {
    /// Main RAM (32 MB)
    ram: Box<[u8]>,
    /// Heap pointer (next allocation address)
    heap_ptr: u32,
    /// Heap allocations (addr -> size)
    allocations: std::collections::HashMap<u32, u32>,
}

impl Memory {
    /// Create a new memory instance with all RAM zeroed
    pub fn new() -> Self {
        // Heap starts in the middle of RAM (16MB offset)
        Self {
            ram: vec![0u8; RAM_SIZE as usize].into_boxed_slice(),
            heap_ptr: 0x0100_0000, // 16MB
            allocations: std::collections::HashMap::new(),
        }
    }

    /// Translate MIPS virtual address to physical address
    /// Handles KSEG0 (0x80000000-0x9FFFFFFF) and KSEG1 (0xA0000000-0xBFFFFFFF)
    fn translate_address(&self, addr: u32) -> u32 {
        match addr {
            // KSEG0: Cached, maps to 0x00000000-0x1FFFFFFF
            0x8000_0000..=0x9FFF_FFFF => addr & KSEG0_MASK,
            // KSEG1: Uncached, maps to 0x00000000-0x1FFFFFFF
            0xA000_0000..=0xBFFF_FFFF => addr & KSEG0_MASK,
            // KSEG2/KSEG3: Not commonly used, pass through
            _ => addr,
        }
    }

    /// Read a byte from memory
    pub fn read_u8(&self, addr: u32) -> Result<u8> {
        let phys_addr = self.translate_address(addr);
        if (RAM_BASE..RAM_BASE + RAM_SIZE).contains(&phys_addr) {
            Ok(self.ram[(phys_addr - RAM_BASE) as usize])
        } else {
            Err(SimulatorError::MemoryError {
                addr,
                message: "out of bounds".to_string(),
            })
        }
    }

    /// Read a 16-bit value from memory (little-endian)
    pub fn read_u16(&self, addr: u32) -> Result<u16> {
        let b0 = self.read_u8(addr)? as u16;
        let b1 = self.read_u8(addr.wrapping_add(1))? as u16;
        Ok(b0 | (b1 << 8))
    }

    /// Read a 32-bit value from memory (little-endian)
    pub fn read_u32(&self, addr: u32) -> Result<u32> {
        let b0 = self.read_u8(addr)? as u32;
        let b1 = self.read_u8(addr.wrapping_add(1))? as u32;
        let b2 = self.read_u8(addr.wrapping_add(2))? as u32;
        let b3 = self.read_u8(addr.wrapping_add(3))? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    /// Write a byte to memory
    pub fn write_u8(&mut self, addr: u32, value: u8) -> Result<()> {
        let phys_addr = self.translate_address(addr);
        if (RAM_BASE..RAM_BASE + RAM_SIZE).contains(&phys_addr) {
            self.ram[(phys_addr - RAM_BASE) as usize] = value;
            Ok(())
        } else {
            Err(SimulatorError::MemoryError {
                addr,
                message: "out of bounds".to_string(),
            })
        }
    }

    /// Write a 16-bit value to memory (little-endian)
    pub fn write_u16(&mut self, addr: u32, value: u16) -> Result<()> {
        self.write_u8(addr, value as u8)?;
        self.write_u8(addr.wrapping_add(1), (value >> 8) as u8)?;
        Ok(())
    }

    /// Write a 32-bit value to memory (little-endian)
    pub fn write_u32(&mut self, addr: u32, value: u32) -> Result<()> {
        self.write_u8(addr, value as u8)?;
        self.write_u8(addr.wrapping_add(1), (value >> 8) as u8)?;
        self.write_u8(addr.wrapping_add(2), (value >> 16) as u8)?;
        self.write_u8(addr.wrapping_add(3), (value >> 24) as u8)?;
        Ok(())
    }

    /// Load data into memory at the specified address
    pub fn load_data(&mut self, addr: u32, data: &[u8]) -> Result<()> {
        for (i, &byte) in data.iter().enumerate() {
            self.write_u8(addr.wrapping_add(i as u32), byte)?;
        }
        Ok(())
    }

    /// Get a slice of memory (for direct access)
    pub fn as_slice(&self) -> &[u8] {
        &self.ram
    }

    /// Get a mutable slice of memory (for direct access)
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.ram
    }

    /// Allocate memory from the heap
    pub fn malloc(&mut self, size: u32) -> u32 {
        if size == 0 {
            return 0;
        }

        // Align to 4 bytes
        let aligned_size = (size + 3) & !3;
        let ptr = self.heap_ptr;

        // Check if allocation would exceed RAM
        if ptr + aligned_size > RAM_BASE + RAM_SIZE {
            log::warn!("malloc failed: not enough memory");
            return 0;
        }

        self.heap_ptr = self.heap_ptr.wrapping_add(aligned_size);
        self.allocations.insert(ptr, aligned_size);
        ptr
    }

    /// Free previously allocated memory
    pub fn free(&mut self, ptr: u32) {
        if ptr != 0 {
            self.allocations.remove(&ptr);
        }
    }

    /// Reallocate memory
    pub fn realloc(&mut self, ptr: u32, new_size: u32) -> u32 {
        if ptr == 0 {
            return self.malloc(new_size);
        }

        if new_size == 0 {
            self.free(ptr);
            return 0;
        }

        // Allocate new block and copy data
        let new_ptr = self.malloc(new_size);
        if let Some(&old_size) = self.allocations.get(&ptr) {
            let copy_size = old_size.min(new_size);
            // Copy old data to new location
            let old_data: Vec<u8> = (0..copy_size)
                .filter_map(|i| self.read_u8(ptr.wrapping_add(i)).ok())
                .collect();
            for (i, &byte) in old_data.iter().enumerate() {
                let _ = self.write_u8(new_ptr.wrapping_add(i as u32), byte);
            }
        }
        self.free(ptr);
        new_ptr
    }

    /// Set memory to a value
    pub fn memset(&mut self, ptr: u32, value: u8, size: u32) {
        for i in 0..size {
            let _ = self.write_u8(ptr.wrapping_add(i), value);
        }
    }

    /// Copy memory (handles overlapping regions)
    pub fn memcpy(&mut self, dest: u32, src: u32, size: u32) -> Result<()> {
        // Read source data first to handle overlapping regions
        let data: Vec<u8> = (0..size)
            .filter_map(|i| self.read_u8(src.wrapping_add(i)).ok())
            .collect();

        for (i, &byte) in data.iter().enumerate() {
            self.write_u8(dest.wrapping_add(i as u32), byte)?;
        }
        Ok(())
    }

    /// Read a null-terminated string length
    pub fn read_string_len(&self, ptr: u32) -> u32 {
        let mut len = 0;
        while let Ok(b) = self.read_u8(ptr.wrapping_add(len)) {
            if b == 0 {
                break;
            }
            len += 1;
        }
        len
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_read_write() {
        let mut mem = Memory::new();
        mem.write_u8(0x0000_0000, 0xAB).unwrap();
        assert_eq!(mem.read_u8(0x0000_0000).unwrap(), 0xAB);
    }

    #[test]
    fn test_memory_u16() {
        let mut mem = Memory::new();
        mem.write_u16(0x0000_0000, 0x1234).unwrap();
        assert_eq!(mem.read_u16(0x0000_0000).unwrap(), 0x1234);
    }

    #[test]
    fn test_memory_u32() {
        let mut mem = Memory::new();
        mem.write_u32(0x0000_0000, 0x1234_5678).unwrap();
        assert_eq!(mem.read_u32(0x0000_0000).unwrap(), 0x1234_5678);
    }

    #[test]
    fn test_memory_bounds_check() {
        let mem = Memory::new();
        assert!(mem.read_u8(RAM_SIZE).is_err());
    }
}
