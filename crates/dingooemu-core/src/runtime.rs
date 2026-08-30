use crate::app_loader::{AppImage, PackageImage};
use crate::audio::AudioConfig;
use crate::cheats::{CheatParseError, CheatRule};
use crate::content::{ArmProfile, ContentFormat, GuestArchitecture};
use crate::cpu::UnknownInstructionPolicy;
use crate::emulator::{Emulator as AppRuntime, JitDiagnostics, UnknownHleCall, UnknownHlePolicy};
use crate::error::{Result, SimulatorError};
use std::path::{Path, PathBuf};

/// Architecture-specific runtime selected by the content probe.
enum Runtime {
    App(AppRuntime),
}

/// Format-neutral emulator façade used by every frontend.
pub struct Emulator {
    runtime: Runtime,
}

impl Default for Emulator {
    fn default() -> Self {
        Self {
            runtime: Runtime::App(AppRuntime::default()),
        }
    }
}

impl Emulator {
    /// Load content, validate its architecture, and select a runtime.
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let package = PackageImage::from_path(path)?;
        Self::from_package_with_path(package, path.to_string_lossy().into_owned())
    }

    /// Create an emulator from an already parsed package.
    pub fn from_app(package: AppImage) -> Result<Self> {
        Self::from_package_with_path(package, String::new())
    }

    fn from_package_with_path(package: PackageImage, path: String) -> Result<Self> {
        let runtime = match package.architecture() {
            GuestArchitecture::Mips32 => {
                Runtime::App(AppRuntime::from_app_with_path(package, path)?)
            }
            GuestArchitecture::Arm32 => {
                return Err(SimulatorError::UnsupportedContentFormat(format!(
                    ".{} requires the ARM runtime, which is not available yet",
                    package.format()
                )));
            }
        };
        Ok(Self { runtime })
    }

    fn app_runtime(&self) -> &AppRuntime {
        match &self.runtime {
            Runtime::App(runtime) => runtime,
        }
    }

    fn app_mut(&mut self) -> &mut AppRuntime {
        match &mut self.runtime {
            Runtime::App(runtime) => runtime,
        }
    }

    pub fn start(&mut self) {
        self.app_mut().start();
    }

    pub fn stop(&mut self) {
        self.app_mut().stop();
    }

    pub fn reset(&mut self) -> Result<()> {
        self.app_mut().reset()
    }

    pub fn tick(&mut self) -> Result<()> {
        self.app_mut().tick()
    }

    pub fn is_running(&self) -> bool {
        self.app_runtime().is_running()
    }

    pub fn set_unknown_hle_policy(&mut self, policy: UnknownHlePolicy) {
        self.app_mut().set_unknown_hle_policy(policy);
    }

    pub fn set_unknown_hle_allowlist<I, S>(&mut self, names: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.app_mut().set_unknown_hle_allowlist(names);
    }

    pub fn unknown_hle_calls(&self) -> impl ExactSizeIterator<Item = &UnknownHleCall> {
        self.app_runtime().unknown_hle_calls()
    }

    pub fn clear_unknown_hle_calls(&mut self) {
        self.app_mut().clear_unknown_hle_calls();
    }

    pub fn serialized_state_size(&self) -> usize {
        self.app_runtime().serialized_state_size()
    }

    pub fn serialize_state(&self, output: &mut [u8]) -> anyhow::Result<()> {
        self.app_runtime().serialize_state(output)
    }

    pub fn unserialize_state(&mut self, input: &[u8]) -> anyhow::Result<()> {
        self.app_mut().unserialize_state(input)
    }

    pub fn set_jit_enabled(&mut self, enabled: bool) {
        self.app_mut().set_jit_enabled(enabled);
    }

    pub fn set_jit_diagnostics_enabled(&mut self, enabled: bool) {
        self.app_mut().set_jit_diagnostics_enabled(enabled);
    }

    pub fn jit_diagnostics(&self) -> JitDiagnostics {
        self.app_runtime().jit_diagnostics()
    }

    pub fn flush_save_files(&mut self) {
        self.app_mut().flush_save_files();
    }

    pub fn set_save_directory<P: Into<PathBuf>>(&mut self, directory: P) {
        self.app_mut().set_save_directory(directory);
    }

    pub fn set_buttons(&mut self, buttons: u32) {
        self.app_mut().set_buttons(buttons);
    }

    pub fn content_format(&self) -> ContentFormat {
        self.app_runtime().content_format()
    }

    pub fn guest_architecture(&self) -> GuestArchitecture {
        self.app_runtime().guest_architecture()
    }

    pub fn arm_profile(&self) -> Option<ArmProfile> {
        self.app_runtime().arm_profile()
    }

    pub fn set_unknown_instruction_policy(&mut self, policy: UnknownInstructionPolicy) {
        self.app_mut().set_unknown_instruction_policy(policy);
    }

    pub fn instruction_count(&self) -> u64 {
        self.app_runtime().instruction_count()
    }

    pub fn set_master_volume(&mut self, volume: u8) {
        self.app_mut().set_master_volume(volume);
    }

    #[cfg(feature = "standalone")]
    pub fn set_host_audio_output_enabled(&mut self, enabled: bool) {
        self.app_mut().set_host_audio_output_enabled(enabled);
    }

    pub fn audio_config(&self) -> Option<AudioConfig> {
        self.app_runtime().audio_config()
    }

    pub fn set_input_repeat_timing(&mut self, delay: u32, period: u32) {
        self.app_mut().set_input_repeat_timing(delay, period);
    }

    pub fn set_swap_ab(&mut self, swap_ab: bool) {
        self.app_mut().set_swap_ab(swap_ab);
    }

    pub fn buttons(&self) -> u32 {
        self.app_runtime().buttons()
    }

    pub fn framebuffer(&self) -> &[u8] {
        self.app_runtime().framebuffer()
    }

    pub fn framebuffer_crc32(&self) -> u32 {
        self.app_runtime().framebuffer_crc32()
    }

    pub fn frame_xrgb8888(&self) -> Vec<u32> {
        self.app_runtime().frame_xrgb8888()
    }

    pub fn save_screenshot(&self, path: &Path) -> anyhow::Result<()> {
        self.app_runtime().save_screenshot(path)
    }

    pub fn system_ram(&self) -> &[u8] {
        self.app_runtime().system_ram()
    }

    pub fn system_ram_mut(&mut self) -> &mut [u8] {
        self.app_mut().system_ram_mut()
    }

    pub fn video_ram(&self) -> &[u8] {
        self.app_runtime().video_ram()
    }

    pub fn video_ram_mut(&mut self) -> &mut [u8] {
        self.app_mut().video_ram_mut()
    }

    pub fn read_memory_u32(&self, address: u32) -> Result<u32> {
        self.app_runtime().read_memory_u32(address)
    }

    pub fn write_memory_u32(&mut self, address: u32, value: u32) -> Result<()> {
        self.app_mut().write_memory_u32(address, value)
    }

    pub fn set_cheat(
        &mut self,
        index: u32,
        enabled: bool,
        code: &str,
    ) -> std::result::Result<(), CheatParseError> {
        self.app_mut().set_cheat(index, enabled, code)
    }

    pub fn set_parsed_cheat(
        &mut self,
        index: u32,
        enabled: bool,
        rule: CheatRule,
    ) -> std::result::Result<(), CheatParseError> {
        self.app_mut().set_parsed_cheat(index, enabled, rule)
    }

    pub fn clear_cheats(&mut self) {
        self.app_mut().clear_cheats();
    }

    pub fn take_audio_samples(&mut self) -> Vec<i16> {
        self.app_mut().take_audio_samples()
    }

    pub fn audio_sample_rate(&self) -> u32 {
        self.app_runtime().audio_sample_rate()
    }

    pub fn frame_count(&self) -> u64 {
        self.app_runtime().frame_count()
    }

    pub fn app(&self) -> Option<&PackageImage> {
        self.app_runtime().app()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_app() -> PackageImage {
        let mut data = vec![0u8; 256];
        data[0..4].copy_from_slice(b"CCDL");
        data[0x20..0x24].copy_from_slice(b"IMPT");
        data[0x40..0x44].copy_from_slice(b"EXPT");
        data[0x60..0x64].copy_from_slice(b"RAWD");
        data[0x68..0x6c].copy_from_slice(&0x80u32.to_le_bytes());
        data[0x6c..0x70].copy_from_slice(&0x20u32.to_le_bytes());
        data[0x74..0x78].copy_from_slice(&0x80a0_0000u32.to_le_bytes());
        data[0x78..0x7c].copy_from_slice(&0x80a0_0000u32.to_le_bytes());
        data[0x7c..0x80].copy_from_slice(&0x20u32.to_le_bytes());
        PackageImage::parse(&data).unwrap()
    }

    #[test]
    fn app_package_selects_the_mips_runtime() {
        let emulator = Emulator::from_app(minimal_app()).unwrap();

        assert_eq!(emulator.content_format(), ContentFormat::App);
        assert_eq!(emulator.guest_architecture(), GuestArchitecture::Mips32);
        assert_eq!(emulator.arm_profile(), None);
    }

    #[test]
    fn arm_package_requires_an_arm_runtime_variant() {
        let mut package = minimal_app();
        package.format = ContentFormat::Cc;
        package.rawd.entry = ArmProfile::RETAIL_ORIGIN;
        package.rawd.origin = ArmProfile::RETAIL_ORIGIN;

        assert!(matches!(
            Emulator::from_app(package),
            Err(SimulatorError::UnsupportedContentFormat(_))
        ));
    }
}
