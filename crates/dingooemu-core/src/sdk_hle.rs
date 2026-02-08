use crate::cpu::Cpu;
use crate::error::Result;
use crate::memory::Memory;
use std::collections::HashMap;

/// SDK HLE (High-Level Emulation) bridge for Dingoo A320
///
/// This module implements the Dingoo SDK functions that games call via
/// the import table. Instead of emulating the actual firmware, we intercept
/// these calls and provide equivalent functionality.
pub struct SdkHle {
    /// SDK call log for debugging
    call_log: Vec<SdkCall>,
    /// Function name -> ID mapping
    name_to_id: HashMap<String, u32>,
    /// Function ID -> handler mapping
    handlers: HashMap<u32, SdkHandler>,
}

/// SDK call record
#[derive(Debug, Clone)]
pub struct SdkCall {
    pub addr: u32,
    pub function_id: u32,
    pub name: String,
    pub timestamp: u64,
}

/// SDK function handler type
type SdkHandler = fn(&mut SdkContext) -> Result<()>;

/// Context for SDK function execution
pub struct SdkContext<'a> {
    pub cpu: &'a mut Cpu,
    pub memory: &'a mut Memory,
    pub video_addr: u32,
    pub input_buttons: u32,
    pub tick_count: u64,
}

impl SdkHle {
    /// Create a new SDK HLE bridge
    pub fn new() -> Self {
        let mut sdk = Self {
            call_log: Vec::new(),
            name_to_id: HashMap::new(),
            handlers: HashMap::new(),
        };

        sdk.register_handlers();
        sdk
    }

    /// Register all SDK function handlers
    fn register_handlers(&mut self) {
        // Memory management
        self.register("malloc", 0x100, Self::handle_malloc);
        self.register("free", 0x101, Self::handle_free);
        self.register("realloc", 0x102, Self::handle_realloc);
        self.register("memset", 0x103, Self::handle_memset);
        self.register("memcpy", 0x104, Self::handle_memcpy);
        self.register("strlen", 0x105, Self::handle_strlen);

        // Graphics/LCD
        self.register("lcd_get_frame", 0x200, Self::handle_lcd_get_frame);
        self.register("lcd_set_frame", 0x201, Self::handle_lcd_set_frame);
        self.register("lcd_flip", 0x202, Self::handle_lcd_flip);
        self.register("lcd_get_bpp", 0x203, Self::handle_lcd_get_bpp);

        // Input
        self.register("kbd_get_status", 0x300, Self::handle_kbd_get_status);
        self.register("kbd_get_key", 0x301, Self::handle_kbd_get_key);

        // Timer
        self.register("GetTickCount", 0x400, Self::handle_get_tick_count);
        self.register("delay_ms", 0x401, Self::handle_delay_ms);
        self.register("OSTimeGet", 0x402, Self::handle_ostime_get);

        // File I/O (stubs)
        self.register("fopen", 0x500, Self::handle_fopen_stub);
        self.register("fclose", 0x501, Self::handle_fclose_stub);
        self.register("fread", 0x502, Self::handle_fread_stub);
        self.register("fwrite", 0x503, Self::handle_fwrite_stub);
    }

    /// Register a handler
    fn register(&mut self, name: &str, id: u32, handler: SdkHandler) {
        self.name_to_id.insert(name.to_string(), id);
        self.handlers.insert(id, handler);
    }

    /// Handle an SDK call
    pub fn handle_call(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut Memory,
        addr: u32,
        function_id: u32,
    ) -> Result<()> {
        // Get function name for logging
        let name = self
            .name_to_id
            .iter()
            .find(|(_, &id)| id == function_id)
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| format!("unknown_{:#x}", function_id));

        // Log the call
        self.call_log.push(SdkCall {
            addr,
            function_id,
            name: name.clone(),
            timestamp: 0,
        });

        // Create context
        let mut ctx = SdkContext {
            cpu,
            memory,
            video_addr: 0x0300_0000,
            input_buttons: 0,
            tick_count: 0,
        };

        // Dispatch to handler
        if let Some(handler) = self.handlers.get(&function_id) {
            handler(&mut ctx)?;
        } else {
            log::warn!(
                "Unimplemented SDK call: {} ({:#04x}) at {:#010x}",
                name,
                function_id,
                addr
            );
            // Return 0 in $v0 for unimplemented calls
            ctx.cpu.regs.write(2, 0);
        }

        Ok(())
    }

    /// Get the call log
    pub fn call_log(&self) -> &[SdkCall] {
        &self.call_log
    }

    /// Clear the call log
    pub fn clear_log(&mut self) {
        self.call_log.clear();
    }

    // ========================================================================
    // Memory Management Handlers
    // ========================================================================

    /// malloc(size) -> ptr
    fn handle_malloc(ctx: &mut SdkContext) -> Result<()> {
        let size = ctx.cpu.regs.read(4); // $a0 = size
        let ptr = ctx.memory.malloc(size);
        ctx.cpu.regs.write(2, ptr); // $v0 = ptr
        log::trace!("SDK: malloc({}) = {:#010x}", size, ptr);
        Ok(())
    }

    /// free(ptr)
    fn handle_free(ctx: &mut SdkContext) -> Result<()> {
        let ptr = ctx.cpu.regs.read(4); // $a0 = ptr
        ctx.memory.free(ptr);
        log::trace!("SDK: free({:#010x})", ptr);
        Ok(())
    }

    /// realloc(ptr, size) -> ptr
    fn handle_realloc(ctx: &mut SdkContext) -> Result<()> {
        let ptr = ctx.cpu.regs.read(4); // $a0 = ptr
        let size = ctx.cpu.regs.read(5); // $a1 = size
        let new_ptr = ctx.memory.realloc(ptr, size);
        ctx.cpu.regs.write(2, new_ptr); // $v0 = new_ptr
        log::trace!("SDK: realloc({:#010x}, {}) = {:#010x}", ptr, size, new_ptr);
        Ok(())
    }

    /// memset(ptr, value, size) -> ptr
    fn handle_memset(ctx: &mut SdkContext) -> Result<()> {
        let ptr = ctx.cpu.regs.read(4); // $a0 = ptr
        let value = ctx.cpu.regs.read(5) as u8; // $a1 = value
        let size = ctx.cpu.regs.read(6); // $a2 = size
        ctx.memory.memset(ptr, value, size);
        ctx.cpu.regs.write(2, ptr); // $v0 = ptr
        log::trace!("SDK: memset({:#010x}, {:#04x}, {})", ptr, value, size);
        Ok(())
    }

    /// memcpy(dest, src, size) -> dest
    fn handle_memcpy(ctx: &mut SdkContext) -> Result<()> {
        let dest = ctx.cpu.regs.read(4); // $a0 = dest
        let src = ctx.cpu.regs.read(5); // $a1 = src
        let size = ctx.cpu.regs.read(6); // $a2 = size
        ctx.memory.memcpy(dest, src, size)?;
        ctx.cpu.regs.write(2, dest); // $v0 = dest
        log::trace!("SDK: memcpy({:#010x}, {:#010x}, {})", dest, src, size);
        Ok(())
    }

    /// strlen(ptr) -> len
    fn handle_strlen(ctx: &mut SdkContext) -> Result<()> {
        let ptr = ctx.cpu.regs.read(4); // $a0 = ptr
        let len = ctx.memory.read_string_len(ptr);
        ctx.cpu.regs.write(2, len); // $v0 = len
        log::trace!("SDK: strlen({:#010x}) = {}", ptr, len);
        Ok(())
    }

    // ========================================================================
    // Graphics/LCD Handlers
    // ========================================================================

    /// lcd_get_frame() -> framebuffer_ptr
    fn handle_lcd_get_frame(ctx: &mut SdkContext) -> Result<()> {
        ctx.cpu.regs.write(2, ctx.video_addr); // $v0 = addr
        log::trace!("SDK: lcd_get_frame() = {:#010x}", ctx.video_addr);
        Ok(())
    }

    /// lcd_set_frame(addr)
    fn handle_lcd_set_frame(ctx: &mut SdkContext) -> Result<()> {
        let addr = ctx.cpu.regs.read(4); // $a0 = addr
        ctx.video_addr = addr;
        log::trace!("SDK: lcd_set_frame({:#010x})", addr);
        Ok(())
    }

    /// lcd_flip()
    fn handle_lcd_flip(_ctx: &mut SdkContext) -> Result<()> {
        // In a real implementation, this would trigger a framebuffer update
        log::trace!("SDK: lcd_flip()");
        Ok(())
    }

    /// lcd_get_bpp() -> 16 (RGB565)
    fn handle_lcd_get_bpp(ctx: &mut SdkContext) -> Result<()> {
        ctx.cpu.regs.write(2, 16); // $v0 = 16
        log::trace!("SDK: lcd_get_bpp() = 16");
        Ok(())
    }

    // ========================================================================
    // Input Handlers
    // ========================================================================

    /// kbd_get_status(status_ptr)
    fn handle_kbd_get_status(ctx: &mut SdkContext) -> Result<()> {
        let status_ptr = ctx.cpu.regs.read(4); // $a0 = status struct pointer
                                               // Write button state to the status struct
        ctx.memory.write_u32(status_ptr, ctx.input_buttons)?;
        log::trace!(
            "SDK: kbd_get_status({:#010x}) = {:#010x}",
            status_ptr,
            ctx.input_buttons
        );
        Ok(())
    }

    /// kbd_get_key() -> keycode
    fn handle_kbd_get_key(ctx: &mut SdkContext) -> Result<()> {
        // Convert bitmask to key code
        let key = if ctx.input_buttons & 0x0001 != 0 {
            1 // UP
        } else if ctx.input_buttons & 0x0002 != 0 {
            2 // DOWN
        } else if ctx.input_buttons & 0x0004 != 0 {
            3 // LEFT
        } else if ctx.input_buttons & 0x0008 != 0 {
            4 // RIGHT
        } else if ctx.input_buttons & 0x0010 != 0 {
            5 // A
        } else if ctx.input_buttons & 0x0020 != 0 {
            6 // B
        } else if ctx.input_buttons & 0x0040 != 0 {
            7 // X
        } else if ctx.input_buttons & 0x0080 != 0 {
            8 // Y
        } else if ctx.input_buttons & 0x0100 != 0 {
            9 // START
        } else if ctx.input_buttons & 0x0200 != 0 {
            10 // SELECT
        } else if ctx.input_buttons & 0x0400 != 0 {
            11 // L
        } else if ctx.input_buttons & 0x0800 != 0 {
            12 // R
        } else {
            0 // No key
        };

        ctx.cpu.regs.write(2, key); // $v0 = key
        log::trace!("SDK: kbd_get_key() = {}", key);
        Ok(())
    }

    // ========================================================================
    // Timer Handlers
    // ========================================================================

    /// GetTickCount() -> milliseconds
    fn handle_get_tick_count(ctx: &mut SdkContext) -> Result<()> {
        ctx.cpu.regs.write(2, ctx.tick_count as u32); // $v0 = ticks
        log::trace!("SDK: GetTickCount() = {}", ctx.tick_count);
        Ok(())
    }

    /// delay_ms(ms)
    fn handle_delay_ms(ctx: &mut SdkContext) -> Result<()> {
        let ms = ctx.cpu.regs.read(4); // $a0 = ms
                                       // In a real implementation, this would sleep
        log::trace!("SDK: delay_ms({})", ms);
        Ok(())
    }

    /// OSTimeGet() -> OS ticks
    fn handle_ostime_get(ctx: &mut SdkContext) -> Result<()> {
        // OS ticks are typically in microseconds or a custom unit
        ctx.cpu.regs.write(2, (ctx.tick_count * 1000) as u32); // $v0 = ticks * 1000
        log::trace!("SDK: OSTimeGet() = {}", ctx.tick_count * 1000);
        Ok(())
    }

    // ========================================================================
    // File I/O Stubs
    // ========================================================================

    /// fopen(path, mode) -> handle (stub)
    fn handle_fopen_stub(ctx: &mut SdkContext) -> Result<()> {
        let _path_ptr = ctx.cpu.regs.read(4); // $a0 = path
        let _mode_ptr = ctx.cpu.regs.read(5); // $a1 = mode
        ctx.cpu.regs.write(2, 0); // $v0 = 0 (NULL)
        log::trace!("SDK: fopen() = NULL (stub)");
        Ok(())
    }

    /// fclose(handle) (stub)
    fn handle_fclose_stub(ctx: &mut SdkContext) -> Result<()> {
        let _handle = ctx.cpu.regs.read(4); // $a0 = handle
        ctx.cpu.regs.write(2, 0); // $v0 = 0 (success)
        log::trace!("SDK: fclose() = 0 (stub)");
        Ok(())
    }

    /// fread(buf, size, count, handle) -> count (stub)
    fn handle_fread_stub(ctx: &mut SdkContext) -> Result<()> {
        let _buf = ctx.cpu.regs.read(4); // $a0 = buf
        let _size = ctx.cpu.regs.read(5); // $a1 = size
        let _count = ctx.cpu.regs.read(6); // $a2 = count
        let _handle = ctx.cpu.regs.read(7); // $a3 = handle
        ctx.cpu.regs.write(2, 0); // $v0 = 0
        log::trace!("SDK: fread() = 0 (stub)");
        Ok(())
    }

    /// fwrite(buf, size, count, handle) -> count (stub)
    fn handle_fwrite_stub(ctx: &mut SdkContext) -> Result<()> {
        let _buf = ctx.cpu.regs.read(4); // $a0 = buf
        let _size = ctx.cpu.regs.read(5); // $a1 = size
        let _count = ctx.cpu.regs.read(6); // $a2 = count
        let _handle = ctx.cpu.regs.read(7); // $a3 = handle
        ctx.cpu.regs.write(2, 0); // $v0 = 0
        log::trace!("SDK: fwrite() = 0 (stub)");
        Ok(())
    }
}

impl Default for SdkHle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sdk_creation() {
        let sdk = SdkHle::new();
        assert!(sdk.call_log().is_empty());
    }

    #[test]
    fn test_sdk_handler_registration() {
        let sdk = SdkHle::new();
        assert!(sdk.handlers.contains_key(&0x100)); // malloc
        assert!(sdk.handlers.contains_key(&0x200)); // lcd_get_frame
        assert!(sdk.handlers.contains_key(&0x300)); // kbd_get_status
        assert!(sdk.handlers.contains_key(&0x400)); // GetTickCount
    }
}
