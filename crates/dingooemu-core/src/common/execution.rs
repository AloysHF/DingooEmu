/// Behavior when a guest executes an unsupported instruction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum UnknownInstructionPolicy {
    Stop,
    #[default]
    Skip,
}
