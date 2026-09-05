/// Aggregated native translation counters for A320 performance diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JitDiagnostics {
    pub feature_available: bool,
    pub enabled: bool,
    pub backend_available: bool,
    pub tracked_blocks: usize,
    pub compiled_blocks: usize,
    pub failed_blocks: usize,
    pub execute_requests: u64,
    pub native_executions: u64,
    pub native_instructions: u64,
    pub interpreter_executions: u64,
    pub interpreter_instructions: u64,
    pub compilation_attempts: u64,
    pub compilation_failures: u64,
    pub compilation_total_us: u64,
    pub compilation_max_us: u64,
    pub cold_fallbacks: u64,
    pub instruction_limit_fallbacks: u64,
    pub zero_exit_fallbacks: u64,
}
