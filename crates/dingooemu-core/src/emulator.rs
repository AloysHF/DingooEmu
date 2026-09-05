use crate::a320::{runtime::Runtime as A320Runtime, JitDiagnostics};
use crate::a330::runtime::Runtime as A330Runtime;
use crate::common::audio::AudioConfig;
use crate::common::cheats::{CheatParseError, CheatRule};
use crate::common::execution::UnknownInstructionPolicy;
use crate::common::hle::{UnknownHleCall, UnknownHlePolicy};
use crate::content::{ArmProfile, ContentFormat, GuestArchitecture, TargetDevice};
use crate::error::Result;
use crate::package::PackageImage;
use std::path::{Path, PathBuf};

/// Device-specific runtime selected from package metadata.
enum Runtime {
    A320(Box<A320Runtime>),
    A330(Box<A330Runtime>),
}

/// Format-neutral emulator façade used by every frontend.
pub struct Emulator {
    runtime: Runtime,
}

impl Default for Emulator {
    fn default() -> Self {
        Self {
            runtime: Runtime::A320(Box::default()),
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
    pub fn from_package(package: PackageImage) -> Result<Self> {
        Self::from_package_with_path(package, String::new())
    }

    fn from_package_with_path(package: PackageImage, path: String) -> Result<Self> {
        let runtime = match package.target() {
            TargetDevice::DingooA320 => Runtime::A320(Box::new(
                A320Runtime::from_package_with_path(package, path)?,
            )),
            TargetDevice::GemeiA330(_) => Runtime::A330(Box::new(A330Runtime::from_package(
                package,
                PathBuf::from(path),
            )?)),
        };
        Ok(Self { runtime })
    }

    pub fn start(&mut self) {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.start(),
            Runtime::A330(runtime) => runtime.start(),
        }
    }

    pub fn stop(&mut self) {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.stop(),
            Runtime::A330(runtime) => runtime.stop(),
        }
    }

    pub fn reset(&mut self) -> Result<()> {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.reset(),
            Runtime::A330(runtime) => runtime.reset(),
        }
    }

    pub fn tick(&mut self) -> Result<()> {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.tick(),
            Runtime::A330(runtime) => runtime.tick(),
        }
    }

    pub fn is_running(&self) -> bool {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.is_running(),
            Runtime::A330(runtime) => runtime.is_running(),
        }
    }

    pub fn set_unknown_hle_policy(&mut self, policy: UnknownHlePolicy) {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.set_unknown_hle_policy(policy),
            Runtime::A330(runtime) => runtime.set_unknown_hle_policy(policy),
        }
    }

    pub fn set_unknown_hle_allowlist<I, S>(&mut self, names: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let names: Vec<String> = names.into_iter().map(Into::into).collect();
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.set_unknown_hle_allowlist(names),
            Runtime::A330(runtime) => runtime.set_unknown_hle_allowlist(names),
        }
    }

    pub fn unknown_hle_calls(&self) -> Box<dyn ExactSizeIterator<Item = &UnknownHleCall> + '_> {
        match &self.runtime {
            Runtime::A320(runtime) => Box::new(runtime.unknown_hle_calls()),
            Runtime::A330(runtime) => Box::new(runtime.unknown_hle_calls()),
        }
    }

    pub fn clear_unknown_hle_calls(&mut self) {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.clear_unknown_hle_calls(),
            Runtime::A330(runtime) => runtime.clear_unknown_hle_calls(),
        }
    }

    pub fn serialized_state_size(&self) -> usize {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.serialized_state_size(),
            Runtime::A330(runtime) => runtime.serialized_state_size(),
        }
    }

    pub fn serialize_state(&self, output: &mut [u8]) -> anyhow::Result<()> {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.serialize_state(output),
            Runtime::A330(runtime) => runtime.serialize_state(output),
        }
    }

    pub fn unserialize_state(&mut self, input: &[u8]) -> anyhow::Result<()> {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.unserialize_state(input),
            Runtime::A330(runtime) => runtime.unserialize_state(input),
        }
    }

    pub fn set_jit_enabled(&mut self, enabled: bool) {
        if let Runtime::A320(runtime) = &mut self.runtime {
            runtime.set_jit_enabled(enabled);
        }
    }

    pub fn set_jit_diagnostics_enabled(&mut self, enabled: bool) {
        if let Runtime::A320(runtime) = &mut self.runtime {
            runtime.set_jit_diagnostics_enabled(enabled);
        }
    }

    pub fn jit_diagnostics(&self) -> JitDiagnostics {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.jit_diagnostics(),
            Runtime::A330(_) => JitDiagnostics::default(),
        }
    }

    pub fn flush_save_files(&mut self) {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.flush_save_files(),
            Runtime::A330(runtime) => runtime.flush_save_files(),
        }
    }

    pub fn set_save_directory<P: Into<PathBuf>>(&mut self, directory: P) {
        let directory = directory.into();
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.set_save_directory(directory),
            Runtime::A330(runtime) => runtime.set_save_directory(directory),
        }
    }

    pub fn set_buttons(&mut self, buttons: u32) {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.set_buttons(buttons),
            Runtime::A330(runtime) => runtime.input.set_buttons(buttons),
        }
    }

    pub fn content_format(&self) -> ContentFormat {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.content_format(),
            Runtime::A330(runtime) => runtime.format(),
        }
    }

    pub fn guest_architecture(&self) -> GuestArchitecture {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.guest_architecture(),
            Runtime::A330(_) => GuestArchitecture::Arm32,
        }
    }

    pub fn arm_profile(&self) -> Option<ArmProfile> {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.arm_profile(),
            Runtime::A330(runtime) => Some(runtime.profile()),
        }
    }

    pub fn set_unknown_instruction_policy(&mut self, policy: UnknownInstructionPolicy) {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.set_unknown_instruction_policy(policy),
            Runtime::A330(runtime) => runtime.set_unknown_instruction_policy(policy),
        }
    }

    pub fn instruction_count(&self) -> u64 {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.instruction_count(),
            Runtime::A330(runtime) => runtime.cpu.instruction_count,
        }
    }

    pub fn set_master_volume(&mut self, volume: u8) {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.set_master_volume(volume),
            Runtime::A330(runtime) => runtime.audio.set_master_volume(volume),
        }
    }

    #[cfg(feature = "standalone")]
    pub fn set_host_audio_output_enabled(&mut self, enabled: bool) {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.set_host_audio_output_enabled(enabled),
            Runtime::A330(runtime) => runtime.audio.set_host_output_enabled(enabled),
        }
    }

    pub fn audio_config(&self) -> Option<AudioConfig> {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.audio_config(),
            Runtime::A330(runtime) => runtime.audio.config(),
        }
    }

    pub fn set_input_repeat_timing(&mut self, delay: u32, period: u32) {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.set_input_repeat_timing(delay, period),
            Runtime::A330(runtime) => runtime.input.set_repeat_timing(delay, period),
        }
    }

    pub fn set_swap_ab(&mut self, swap_ab: bool) {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.set_swap_ab(swap_ab),
            Runtime::A330(runtime) => runtime.input.set_swap_ab(swap_ab),
        }
    }

    pub fn buttons(&self) -> u32 {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.buttons(),
            Runtime::A330(runtime) => runtime.input.buttons(),
        }
    }

    pub fn framebuffer(&self) -> &[u8] {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.framebuffer(),
            Runtime::A330(runtime) => runtime.video.framebuffer(),
        }
    }

    pub fn framebuffer_crc32(&self) -> u32 {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.framebuffer_crc32(),
            Runtime::A330(runtime) => runtime.video.framebuffer_crc32(),
        }
    }

    pub fn frame_xrgb8888(&self) -> Vec<u32> {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.frame_xrgb8888(),
            Runtime::A330(runtime) => runtime.video.to_xrgb8888(),
        }
    }

    pub fn save_screenshot(&self, path: &Path) -> anyhow::Result<()> {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.save_screenshot(path),
            Runtime::A330(runtime) => runtime.video.save_screenshot(path),
        }
    }

    pub fn system_ram(&self) -> &[u8] {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.system_ram(),
            Runtime::A330(runtime) => runtime.memory.system_ram(),
        }
    }

    pub fn system_ram_guest_address(&self) -> u32 {
        match &self.runtime {
            Runtime::A320(_) => 0,
            Runtime::A330(_) => crate::a330::memory::SYSTEM_RAM_BASE,
        }
    }

    pub fn system_ram_mut(&mut self) -> &mut [u8] {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.system_ram_mut(),
            Runtime::A330(runtime) => runtime.memory.system_ram_mut(),
        }
    }

    pub fn video_ram(&self) -> &[u8] {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.video_ram(),
            Runtime::A330(runtime) => runtime.memory.framebuffer(),
        }
    }

    pub fn video_ram_guest_address(&self) -> u32 {
        match &self.runtime {
            Runtime::A320(_) => crate::a320::memory::LCD_FRAMEBUFFER_BASE,
            Runtime::A330(_) => crate::a330::memory::FRAMEBUFFER_BASE,
        }
    }

    pub fn video_ram_mut(&mut self) -> &mut [u8] {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.video_ram_mut(),
            Runtime::A330(runtime) => runtime.memory.framebuffer_mut(),
        }
    }

    pub fn read_memory_u32(&self, address: u32) -> Result<u32> {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.read_memory_u32(address),
            Runtime::A330(runtime) => runtime.memory.read32(address),
        }
    }

    pub fn write_memory_u32(&mut self, address: u32, value: u32) -> Result<()> {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.write_memory_u32(address, value),
            Runtime::A330(runtime) => runtime.memory.write32(address, value),
        }
    }

    pub fn set_cheat(
        &mut self,
        index: u32,
        enabled: bool,
        code: &str,
    ) -> std::result::Result<(), CheatParseError> {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.set_cheat(index, enabled, code),
            Runtime::A330(runtime) => runtime.set_cheat(index, enabled, code),
        }
    }

    pub fn set_parsed_cheat(
        &mut self,
        index: u32,
        enabled: bool,
        rule: CheatRule,
    ) -> std::result::Result<(), CheatParseError> {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.set_parsed_cheat(index, enabled, rule),
            Runtime::A330(runtime) => runtime.set_parsed_cheat(index, enabled, rule),
        }
    }

    pub fn clear_cheats(&mut self) {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.clear_cheats(),
            Runtime::A330(runtime) => runtime.clear_cheats(),
        }
    }

    pub fn take_audio_samples(&mut self) -> Vec<i16> {
        match &mut self.runtime {
            Runtime::A320(runtime) => runtime.take_audio_samples(),
            Runtime::A330(runtime) => runtime.audio.take_frame_samples(),
        }
    }

    pub fn audio_sample_rate(&self) -> u32 {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.audio_sample_rate(),
            Runtime::A330(_) => crate::common::audio::OUTPUT_SAMPLE_RATE,
        }
    }

    pub fn frame_count(&self) -> u64 {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.frame_count(),
            Runtime::A330(runtime) => runtime.video.frame_count(),
        }
    }

    pub fn package(&self) -> Option<&PackageImage> {
        match &self.runtime {
            Runtime::A320(runtime) => runtime.package(),
            Runtime::A330(runtime) => Some(runtime.package()),
        }
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
        PackageImage::parse(&data, ContentFormat::App).unwrap()
    }

    #[test]
    fn app_package_selects_the_mips_runtime() {
        let emulator = Emulator::from_package(minimal_app()).unwrap();

        assert_eq!(emulator.content_format(), ContentFormat::App);
        assert_eq!(emulator.guest_architecture(), GuestArchitecture::Mips32);
        assert_eq!(emulator.arm_profile(), None);
        assert_eq!(emulator.system_ram_guest_address(), 0);
        assert_eq!(
            emulator.video_ram_guest_address(),
            crate::a320::memory::LCD_FRAMEBUFFER_BASE
        );
    }

    #[test]
    fn arm_package_selects_the_a330_runtime() {
        let mut package = minimal_app();
        package.format = ContentFormat::Cc;
        package.target = TargetDevice::GemeiA330(ArmProfile::Retail);
        package.rawd.entry = ArmProfile::RETAIL_ORIGIN;
        package.rawd.origin = ArmProfile::RETAIL_ORIGIN;

        let emulator = Emulator::from_package(package).unwrap();
        assert_eq!(emulator.content_format(), ContentFormat::Cc);
        assert_eq!(emulator.guest_architecture(), GuestArchitecture::Arm32);
        assert_eq!(emulator.arm_profile(), Some(ArmProfile::Retail));
        assert_eq!(
            emulator.system_ram_guest_address(),
            crate::a330::memory::SYSTEM_RAM_BASE
        );
        assert_eq!(
            emulator.video_ram_guest_address(),
            crate::a330::memory::FRAMEBUFFER_BASE
        );
    }
}
