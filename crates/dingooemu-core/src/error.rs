/// Result type alias for convenience
pub type Result<T> = std::result::Result<T, SimulatorError>;

/// Errors that can occur during emulation
#[derive(Debug, thiserror::Error)]
pub enum SimulatorError {
    /// Invalid .app file format
    #[error("invalid app format: {0}")]
    InvalidAppFormat(String),

    /// Memory access error
    #[error("memory error at {addr:#010x}: {message}")]
    MemoryError { addr: u32, message: String },

    /// CPU execution error
    #[error("CPU error at {pc:#010x}: {message}")]
    CpuError { pc: u32, message: String },

    /// Invalid instruction
    #[error("invalid instruction {instr:#010x} at {pc:#010x}")]
    InvalidInstruction { pc: u32, instr: u32 },

    /// SDK HLE call error
    #[error("SDK HLE error: {0}")]
    SdkHleError(String),

    /// Unknown SDK HLE call rejected by the active policy
    #[error(
        "unknown SDK HLE {name} at {pc:#010x} (import {import_address:#010x}, arguments {arguments:#010x?})"
    )]
    UnknownHle {
        name: String,
        pc: u32,
        import_address: u32,
        arguments: [u32; 4],
    },

    /// I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Other errors
    #[error("{0}")]
    Other(String),
}

impl From<String> for SimulatorError {
    fn from(s: String) -> Self {
        SimulatorError::Other(s)
    }
}

impl From<&str> for SimulatorError {
    fn from(s: &str) -> Self {
        SimulatorError::Other(s.to_string())
    }
}
