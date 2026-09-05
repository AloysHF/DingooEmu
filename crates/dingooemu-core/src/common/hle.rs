/// Behavior when the guest calls an SDK function without an HLE implementation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnknownHlePolicy {
    /// Record the call and return zero to preserve compatibility.
    #[default]
    Report,
    /// Record the call and stop unless the function name is allowlisted.
    Stop,
}

/// Aggregated diagnostics for one unknown SDK HLE function.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct UnknownHleCall {
    pub name: String,
    pub count: u64,
    pub import_address: u32,
    pub first_pc: u32,
    pub first_arguments: [u32; 4],
}
