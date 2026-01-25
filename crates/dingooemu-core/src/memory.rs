use crate::error::{Result, SimulatorError};

/// Dingoo A320 memory regions
const RAM_BASE: u32 = 0x0000_0000;
const RAM_SIZE: u32 = 32 * 1024 * 1024; // 32 MB

/// Memory manager for the Dingoo A320
pub struct Memory {
    /// Main RAM (32 MB)
    ram: Box<[u8]>,
}

impl Memory {
    /// Create a new memory instance with all RAM zeroed
    pub fn new() -> Self {
        Self {
            ram: vec![0u8; RAM_SIZE as usize].into_boxed_slice(),
        }
    }

    /// Read a byte from memory
    pub fn read_u8(&self, addr: u32) -> Result<u8> {
        if (RAM_BASE..RAM_BASE + RAM_SIZE).contains(&addr) {
            Ok(self.ram[(addr - RAM_BASE) as usize])
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
        if (RAM_BASE..RAM_BASE + RAM_SIZE).contains(&addr) {
            self.ram[(addr - RAM_BASE) as usize] = value;
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
