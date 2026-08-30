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

    /// Return the guest instruction set associated with this content category.
    pub const fn architecture(self) -> GuestArchitecture {
        match self {
            Self::App => GuestArchitecture::Mips32,
            Self::Cc | Self::C2s | Self::C3s => GuestArchitecture::Arm32,
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
    fn maps_content_categories_to_guest_architectures() {
        assert_eq!(ContentFormat::App.architecture(), GuestArchitecture::Mips32);
        for format in [ContentFormat::Cc, ContentFormat::C2s, ContentFormat::C3s] {
            assert_eq!(format.architecture(), GuestArchitecture::Arm32);
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
}
