use crate::error::{Result, SimulatorError};
use crate::video::FRAMEBUFFER_MAP_SIZE;

/// Dingoo A320 memory regions
const RAM_BASE: u32 = 0x0000_0000;
const RAM_SIZE: u32 = 32 * 1024 * 1024; // 32 MB

/// MIPS KSEG0 mask (strip top 3 bits for cached segment)
const KSEG0_MASK: u32 = 0x1FFF_FFFF;

/// Guest-visible aliases for the LCD framebuffer mapping.
const LCD_FRAMEBUFFER_ALIASES: [u32; 4] = [
    crate::video::VM_LCD_FB_ADDRESS,
    0x1400_0000,
    0x9000_0000,
    0x1000_0000,
];
const IPU_BASE: u32 = 0x1308_0000;
const IPU_REGISTER_SIZE: usize = 0x100;
const IPU_CTRL_OFFSET: usize = 0x00;
const IPU_STATUS_OFFSET: usize = 0x04;
const IPU_D_FMT_OFFSET: usize = 0x08;
const IPU_Y_ADDR_OFFSET: usize = 0x0C;
const IPU_U_ADDR_OFFSET: usize = 0x10;
const IPU_V_ADDR_OFFSET: usize = 0x14;
const IPU_IN_GS_OFFSET: usize = 0x18;
const IPU_Y_STRIDE_OFFSET: usize = 0x1C;
const IPU_UV_STRIDE_OFFSET: usize = 0x20;
const IPU_OUT_ADDR_OFFSET: usize = 0x24;
const IPU_OUT_GS_OFFSET: usize = 0x28;
const IPU_OUT_STRIDE_OFFSET: usize = 0x2C;
const IPU_CTRL_RUN: u32 = 1 << 1;
const IPU_STATUS_OUT_END: u8 = 1;

/// Memory manager for the Dingoo A320
pub struct Memory {
    /// Main RAM (32 MB)
    ram: Box<[u8]>,
    /// LCD framebuffer memory shared by all guest aliases
    framebuffer: Box<[u8]>,
    /// Image processing unit registers
    ipu_registers: Box<[u8]>,
    /// Heap pointer (next allocation address)
    heap_ptr: u32,
    /// Heap allocations (addr -> size)
    allocations: std::collections::HashMap<u32, u32>,
    /// Reusable free heap blocks (addr, size), sorted by address
    free_blocks: Vec<(u32, u32)>,
    /// Write tracking: addresses that were written to
    write_log: Vec<u32>,
}

impl Memory {
    /// Create a new memory instance with all RAM zeroed
    pub fn new() -> Self {
        // Heap starts in the middle of RAM (16MB offset)
        Self {
            ram: vec![0u8; RAM_SIZE as usize].into_boxed_slice(),
            framebuffer: vec![0u8; FRAMEBUFFER_MAP_SIZE].into_boxed_slice(),
            ipu_registers: {
                let mut registers = vec![0u8; IPU_REGISTER_SIZE].into_boxed_slice();
                registers[IPU_STATUS_OFFSET] = IPU_STATUS_OUT_END;
                registers
            },
            heap_ptr: 0x0100_0000, // 16MB
            allocations: std::collections::HashMap::new(),
            free_blocks: Vec::new(),
            write_log: Vec::new(),
        }
    }

    /// Get a copy of the write log and clear it
    pub fn consume_write_log(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.write_log)
    }

    /// Return the offset inside the LCD framebuffer mapping, if any.
    fn framebuffer_offset(&self, addr: u32) -> Option<usize> {
        LCD_FRAMEBUFFER_ALIASES.iter().find_map(|&base| {
            let offset = addr.wrapping_sub(base);
            (offset < FRAMEBUFFER_MAP_SIZE as u32).then_some(offset as usize)
        })
    }

    fn ipu_register_offset(&self, addr: u32) -> Option<usize> {
        let physical = self.translate_address(addr);
        let offset = physical.wrapping_sub(IPU_BASE);
        (offset < IPU_REGISTER_SIZE as u32).then_some(offset as usize)
    }

    fn ipu_register_u32(&self, offset: usize) -> u32 {
        u32::from_le_bytes([
            self.ipu_registers[offset],
            self.ipu_registers[offset + 1],
            self.ipu_registers[offset + 2],
            self.ipu_registers[offset + 3],
        ])
    }

    fn run_ipu(&mut self) -> Result<()> {
        let format = self.ipu_register_u32(IPU_D_FMT_OFFSET);
        if format & 0x3 != 1 {
            self.ipu_registers[IPU_STATUS_OFFSET] = IPU_STATUS_OUT_END;
            return Ok(());
        }

        let input_geometry = self.ipu_register_u32(IPU_IN_GS_OFFSET);
        let input_width = (input_geometry >> 16) as usize;
        let input_height = (input_geometry & 0xFFFF) as usize;
        let output_geometry = self.ipu_register_u32(IPU_OUT_GS_OFFSET);
        let output_height = (output_geometry & 0xFFFF) as usize;
        let output_stride = self.ipu_register_u32(IPU_OUT_STRIDE_OFFSET) as usize;
        let output_width = output_stride / 4;

        if input_width == 0 || input_height == 0 || output_width == 0 || output_height == 0 {
            self.ipu_registers[IPU_STATUS_OFFSET] = IPU_STATUS_OUT_END;
            return Ok(());
        }

        let y_addr = self.ipu_register_u32(IPU_Y_ADDR_OFFSET);
        let u_addr = self.ipu_register_u32(IPU_U_ADDR_OFFSET);
        let v_addr = self.ipu_register_u32(IPU_V_ADDR_OFFSET);
        let output_addr = self.ipu_register_u32(IPU_OUT_ADDR_OFFSET);
        let y_stride = self.ipu_register_u32(IPU_Y_STRIDE_OFFSET) as usize;
        let uv_stride = self.ipu_register_u32(IPU_UV_STRIDE_OFFSET);
        let u_stride = (uv_stride >> 16) as usize;
        let v_stride = (uv_stride & 0xFFFF) as usize;

        for output_y in 0..output_height {
            let input_y = output_y * input_height / output_height;
            for output_x in 0..output_width {
                let input_x = output_x * input_width / output_width;
                let y = self.read_u8(y_addr.wrapping_add((input_y * y_stride + input_x) as u32))?
                    as i32;
                let u = self
                    .read_u8(u_addr.wrapping_add((input_y * u_stride + input_x / 2) as u32))?
                    as i32
                    - 128;
                let v = self
                    .read_u8(v_addr.wrapping_add((input_y * v_stride + input_x / 2) as u32))?
                    as i32
                    - 128;
                let luma = 1192 * (y - 16).max(0);
                let red = ((luma + 1634 * v) >> 10).clamp(0, 255) as u8;
                let green = ((luma - 400 * u - 833 * v) >> 10).clamp(0, 255) as u8;
                let blue = ((luma + 2066 * u) >> 10).clamp(0, 255) as u8;
                let pixel_addr =
                    output_addr.wrapping_add((output_y * output_stride + output_x * 4) as u32);
                self.write_u8(pixel_addr, blue)?;
                self.write_u8(pixel_addr.wrapping_add(1), green)?;
                self.write_u8(pixel_addr.wrapping_add(2), red)?;
                self.write_u8(pixel_addr.wrapping_add(3), 0)?;
            }
        }

        self.ipu_registers[IPU_STATUS_OFFSET] = IPU_STATUS_OUT_END;
        Ok(())
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
        if let Some(offset) = self.framebuffer_offset(addr) {
            return Ok(self.framebuffer[offset]);
        }
        if let Some(offset) = self.ipu_register_offset(addr) {
            return Ok(self.ipu_registers[offset]);
        }

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
        if let Some(offset) = self.framebuffer_offset(addr) {
            self.framebuffer[offset] = value;
            if self.write_log.len() < 1000 {
                self.write_log.push(addr);
            }
            return Ok(());
        }
        if let Some(offset) = self.ipu_register_offset(addr) {
            self.ipu_registers[offset] = value;
            return Ok(());
        }

        let phys_addr = self.translate_address(addr);
        if (RAM_BASE..RAM_BASE + RAM_SIZE).contains(&phys_addr) {
            self.ram[(phys_addr - RAM_BASE) as usize] = value;
            // Track ALL writes for debugging
            if self.write_log.len() < 1000 {
                self.write_log.push(phys_addr);
            }
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
        let ipu_offset = self.ipu_register_offset(addr);
        if ipu_offset.is_some() {
            log::trace!("IPU write: {:#010x} = {:#010x}", addr, value);
        }
        self.write_u8(addr, value as u8)?;
        self.write_u8(addr.wrapping_add(1), (value >> 8) as u8)?;
        self.write_u8(addr.wrapping_add(2), (value >> 16) as u8)?;
        self.write_u8(addr.wrapping_add(3), (value >> 24) as u8)?;
        if ipu_offset == Some(IPU_CTRL_OFFSET) && value & IPU_CTRL_RUN != 0 {
            self.run_ipu()?;
        }
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

        let Some(aligned_size) = size.checked_add(3).map(|value| value & !3) else {
            log::warn!("malloc failed: invalid allocation size");
            return 0;
        };

        if let Some(index) = self
            .free_blocks
            .iter()
            .position(|&(_, block_size)| block_size >= aligned_size)
        {
            let (ptr, block_size) = self.free_blocks[index];
            if block_size == aligned_size {
                self.free_blocks.remove(index);
            } else {
                self.free_blocks[index] =
                    (ptr.wrapping_add(aligned_size), block_size - aligned_size);
            }
            self.allocations.insert(ptr, aligned_size);
            return ptr;
        }

        let ptr = self.heap_ptr;

        let Some(end) = ptr.checked_add(aligned_size) else {
            log::warn!("malloc failed: not enough memory");
            return 0;
        };
        if end > RAM_BASE + RAM_SIZE {
            log::warn!("malloc failed: not enough memory");
            return 0;
        }

        self.heap_ptr = end;
        self.allocations.insert(ptr, aligned_size);
        ptr
    }

    /// Free previously allocated memory
    pub fn free(&mut self, ptr: u32) {
        let Some(size) = (ptr != 0).then(|| self.allocations.remove(&ptr)).flatten() else {
            return;
        };

        let insert_at = self
            .free_blocks
            .partition_point(|&(address, _)| address < ptr);
        self.free_blocks.insert(insert_at, (ptr, size));

        let mut index = insert_at.saturating_sub(1);
        while index + 1 < self.free_blocks.len() {
            let (address, block_size) = self.free_blocks[index];
            let (next_address, next_size) = self.free_blocks[index + 1];
            if address.checked_add(block_size) != Some(next_address) {
                index += 1;
                continue;
            }
            self.free_blocks[index].1 = block_size + next_size;
            self.free_blocks.remove(index + 1);
        }

        while let Some(&(address, block_size)) = self.free_blocks.last() {
            if address.checked_add(block_size) != Some(self.heap_ptr) {
                break;
            }
            self.heap_ptr = address;
            self.free_blocks.pop();
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

        let Some(&old_size) = self.allocations.get(&ptr) else {
            return 0;
        };
        let Some(aligned_size) = new_size.checked_add(3).map(|value| value & !3) else {
            return 0;
        };
        if aligned_size <= old_size {
            self.allocations.insert(ptr, aligned_size);
            if aligned_size < old_size {
                let tail = ptr.wrapping_add(aligned_size);
                self.allocations.insert(tail, old_size - aligned_size);
                self.free(tail);
            }
            return ptr;
        }

        let new_ptr = self.malloc(new_size);
        if new_ptr == 0 {
            return 0;
        }

        let old_data: Vec<u8> = (0..old_size)
            .filter_map(|i| self.read_u8(ptr.wrapping_add(i)).ok())
            .collect();
        for (i, &byte) in old_data.iter().enumerate() {
            let _ = self.write_u8(new_ptr.wrapping_add(i as u32), byte);
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

    /// Get current heap pointer
    pub fn heap_ptr(&self) -> u32 {
        self.heap_ptr
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
    fn test_framebuffer_aliases_share_storage() {
        let mut mem = Memory::new();

        mem.write_u8(crate::video::VM_LCD_FB_ADDRESS, 0x12).unwrap();
        assert_eq!(mem.read_u8(0x1400_0000).unwrap(), 0x12);

        mem.write_u8(0x9000_0004, 0x34).unwrap();
        assert_eq!(mem.read_u8(0x1000_0004).unwrap(), 0x34);
    }

    #[test]
    fn test_ipu_converts_planar_yuv422_to_rgb888() {
        let mut mem = Memory::new();
        mem.load_data(0x100, &[235; 4]).unwrap();
        mem.load_data(0x110, &[128; 2]).unwrap();
        mem.load_data(0x120, &[128; 2]).unwrap();
        mem.write_u32(IPU_BASE + IPU_D_FMT_OFFSET as u32, 1)
            .unwrap();
        mem.write_u32(IPU_BASE + IPU_Y_ADDR_OFFSET as u32, 0x100)
            .unwrap();
        mem.write_u32(IPU_BASE + IPU_U_ADDR_OFFSET as u32, 0x110)
            .unwrap();
        mem.write_u32(IPU_BASE + IPU_V_ADDR_OFFSET as u32, 0x120)
            .unwrap();
        mem.write_u32(IPU_BASE + IPU_IN_GS_OFFSET as u32, 2 << 16 | 2)
            .unwrap();
        mem.write_u32(IPU_BASE + IPU_Y_STRIDE_OFFSET as u32, 2)
            .unwrap();
        mem.write_u32(IPU_BASE + IPU_UV_STRIDE_OFFSET as u32, 1 << 16 | 1)
            .unwrap();
        mem.write_u32(IPU_BASE + IPU_OUT_ADDR_OFFSET as u32, 0x200)
            .unwrap();
        mem.write_u32(IPU_BASE + IPU_OUT_GS_OFFSET as u32, 2 << 16 | 2)
            .unwrap();
        mem.write_u32(IPU_BASE + IPU_OUT_STRIDE_OFFSET as u32, 8)
            .unwrap();

        mem.write_u32(IPU_BASE + IPU_CTRL_OFFSET as u32, IPU_CTRL_RUN)
            .unwrap();

        assert_eq!(
            (0..4)
                .map(|offset| mem.read_u8(0x200 + offset).unwrap())
                .collect::<Vec<_>>(),
            [254, 254, 254, 0]
        );
        assert_eq!(
            mem.read_u32(IPU_BASE + IPU_STATUS_OFFSET as u32).unwrap()
                & u32::from(IPU_STATUS_OUT_END),
            u32::from(IPU_STATUS_OUT_END)
        );
    }

    #[test]
    fn test_malloc_reuses_and_splits_freed_blocks() {
        let mut mem = Memory::new();
        let first = mem.malloc(64);
        let second = mem.malloc(32);

        mem.free(first);

        assert_eq!(mem.malloc(48), first);
        assert_eq!(mem.malloc(16), first + 48);
        assert_eq!(mem.malloc(4), second + 32);
    }

    #[test]
    fn test_free_coalesces_adjacent_blocks() {
        let mut mem = Memory::new();
        let first = mem.malloc(32);
        let second = mem.malloc(32);
        let _guard = mem.malloc(4);

        mem.free(first);
        mem.free(second);

        assert_eq!(mem.malloc(64), first);
    }

    #[test]
    fn test_realloc_failure_preserves_original_block() {
        let mut mem = Memory::new();
        let ptr = mem.malloc(16);
        mem.write_u32(ptr, 0x1234_5678).unwrap();

        assert_eq!(mem.realloc(ptr, RAM_SIZE), 0);
        assert_eq!(mem.read_u32(ptr).unwrap(), 0x1234_5678);
        assert_eq!(mem.allocations.get(&ptr), Some(&16));
    }

    #[test]
    fn test_memory_bounds_check() {
        let mem = Memory::new();
        assert!(mem.read_u8(RAM_SIZE).is_err());
    }
}
