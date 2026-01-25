use crate::error::Result;
use crate::memory::Memory;

/// SDK HLE (High-Level Emulation) bridge for Dingoo A320
///
/// This module implements the Dingoo SDK functions that games call via
/// the import table. Instead of emulating the actual firmware, we intercept
/// these calls and provide equivalent functionality.
pub struct SdkHle {
    /// SDK call log for debugging
    call_log: Vec<SdkCall>,
}

#[derive(Debug, Clone)]
pub struct SdkCall {
    pub addr: u32,
    pub function_id: u32,
    pub timestamp: u64,
}

impl SdkHle {
    /// Create a new SDK HLE bridge
    pub fn new() -> Self {
        Self {
            call_log: Vec::new(),
        }
    }

    /// Handle an SDK call
    pub fn handle_call(&mut self, addr: u32, function_id: u32, _memory: &Memory) -> Result<()> {
        // Log the call
        self.call_log.push(SdkCall {
            addr,
            function_id,
            timestamp: 0, // TODO: Use actual timestamp
        });

        match function_id {
            // TODO: Implement actual SDK functions
            0x01 => {
                log::trace!("SDK: SubmitFramebuffer at {:#010x}", addr);
                // Framebuffer submission
            }
            0x02 => {
                log::trace!("SDK: GetInput at {:#010x}", addr);
                // Input polling
            }
            0x03 => {
                log::trace!("SDK: PlayAudio at {:#010x}", addr);
                // Audio playback
            }
            _ => {
                log::warn!(
                    "Unimplemented SDK call {:#04x} at {:#010x}",
                    function_id,
                    addr
                );
            }
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
}
