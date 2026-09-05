use std::fmt;
use std::path::Path;

/// User-visible content categories supported by the package probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ContentFormat {
    /// Dingoo A320 application package.
    App,
    /// Gemei A330 firmware 1.0 native content.
    Cc,
    /// Gemei A330 2D native content used by later firmware.
    C2s,
    /// Gemei A330 3D native content used by later firmware.
    C3s,
}

impl ContentFormat {
    /// Detect a supported content category from a case-insensitive extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("app") {
            Some(Self::App)
        } else if extension.eq_ignore_ascii_case("cc") {
            Some(Self::Cc)
        } else if extension.eq_ignore_ascii_case("c2s") {
            Some(Self::C2s)
        } else if extension.eq_ignore_ascii_case("c3s") {
            Some(Self::C3s)
        } else {
            None
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Cc => "cc",
            Self::C2s => "c2s",
            Self::C3s => "c3s",
        }
    }
}

impl fmt::Display for ContentFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.extension())
    }
}

/// CPU architecture selected by validated content metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuestArchitecture {
    Mips32,
    Arm32,
}

/// Device and ABI target detected from validated package metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TargetDevice {
    DingooA320,
    GemeiA330(ArmProfile),
}

impl TargetDevice {
    /// Detect a supported device from the package load address.
    pub const fn detect(origin: u32) -> Option<Self> {
        if let Some(profile) = ArmProfile::detect(origin) {
            Some(Self::GemeiA330(profile))
        } else if origin & 0x8000_0000 != 0 {
            Some(Self::DingooA320)
        } else {
            None
        }
    }

    pub const fn architecture(self) -> GuestArchitecture {
        match self {
            Self::DingooA320 => GuestArchitecture::Mips32,
            Self::GemeiA330(_) => GuestArchitecture::Arm32,
        }
    }

    pub const fn arm_profile(self) -> Option<ArmProfile> {
        match self {
            Self::DingooA320 => None,
            Self::GemeiA330(profile) => Some(profile),
        }
    }
}

impl ContentFormat {
    /// Check whether the declared file category can contain the detected device target.
    pub const fn supports_target(self, target: TargetDevice) -> bool {
        matches!(
            (self, target),
            (Self::App, TargetDevice::DingooA320)
                | (Self::Cc | Self::C2s | Self::C3s, TargetDevice::GemeiA330(_))
        )
    }
}

/// Known Gemei A330 address-space and ABI layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ArmProfile {
    /// Retail SDK layout used by packages loaded at 0x10100000.
    Retail,
    /// Older/homebrew SDK layout used by packages loaded at 0x13800000.
    Homebrew,
}

impl ArmProfile {
    pub const RETAIL_ORIGIN: u32 = 0x1010_0000;
    pub const HOMEBREW_ORIGIN: u32 = 0x1380_0000;

    pub const fn detect(origin: u32) -> Option<Self> {
        match origin {
            Self::RETAIL_ORIGIN => Some(Self::Retail),
            Self::HOMEBREW_ORIGIN => Some(Self::Homebrew),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_extensions_case_insensitively() {
        let cases = [
            ("game.APP", ContentFormat::App),
            ("game.Cc", ContentFormat::Cc),
            ("game.c2S", ContentFormat::C2s),
            ("game.C3S", ContentFormat::C3s),
        ];
        for (path, expected) in cases {
            assert_eq!(ContentFormat::from_path(Path::new(path)), Some(expected));
        }
        assert_eq!(ContentFormat::from_path(Path::new("game.c2m")), None);
        assert_eq!(ContentFormat::from_path(Path::new("game.sim")), None);
    }

    #[test]
    fn validates_content_categories_against_detected_targets() {
        assert!(ContentFormat::App.supports_target(TargetDevice::DingooA320));
        assert!(!ContentFormat::App.supports_target(TargetDevice::GemeiA330(ArmProfile::Retail)));
        for format in [ContentFormat::Cc, ContentFormat::C2s, ContentFormat::C3s] {
            assert!(format.supports_target(TargetDevice::GemeiA330(ArmProfile::Retail)));
            assert!(!format.supports_target(TargetDevice::DingooA320));
        }
    }

    #[test]
    fn detects_only_validated_arm_profiles() {
        assert_eq!(
            ArmProfile::detect(ArmProfile::RETAIL_ORIGIN),
            Some(ArmProfile::Retail)
        );
        assert_eq!(
            ArmProfile::detect(ArmProfile::HOMEBREW_ORIGIN),
            Some(ArmProfile::Homebrew)
        );
        assert_eq!(ArmProfile::detect(0x11c0_0000), None);
    }

    #[test]
    fn detects_target_device_from_load_address() {
        assert_eq!(
            TargetDevice::detect(0x80a0_0000),
            Some(TargetDevice::DingooA320)
        );
        assert_eq!(
            TargetDevice::detect(ArmProfile::RETAIL_ORIGIN),
            Some(TargetDevice::GemeiA330(ArmProfile::Retail))
        );
        assert_eq!(
            TargetDevice::detect(ArmProfile::HOMEBREW_ORIGIN),
            Some(TargetDevice::GemeiA330(ArmProfile::Homebrew))
        );
        assert_eq!(TargetDevice::detect(0x11c0_0000), None);
    }
}
