use std::str::FromStr;

use dingooemu_core::Emulator;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Memory8(u32),
    Memory16(u32),
    Memory32(u32),
    Register(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheatRule {
    target: Target,
    value: u32,
}

impl CheatRule {
    pub fn apply(self, emulator: &mut Emulator) -> dingooemu_core::error::Result<()> {
        match self.target {
            Target::Memory8(address) => emulator.memory.write_u8(address, self.value as u8),
            Target::Memory16(address) => emulator.memory.write_u16(address, self.value as u16),
            Target::Memory32(address) => emulator.memory.write_u32(address, self.value),
            Target::Register(index) => {
                emulator.cpu.regs.write(index, self.value);
                Ok(())
            }
        }
    }
}

impl FromStr for CheatRule {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (target, value) = value
            .split_once('=')
            .ok_or_else(|| "expected TARGET=VALUE".to_string())?;
        let value = parse_number(value.trim())?;
        let (kind, location) = target.split_once(':').ok_or_else(|| {
            "expected mem8:ADDRESS, mem16:ADDRESS, mem32:ADDRESS, or reg:rN".to_string()
        })?;
        let target = match kind.to_ascii_lowercase().as_str() {
            "mem8" => Target::Memory8(parse_number(location)?),
            "mem16" => Target::Memory16(parse_number(location)?),
            "mem32" => Target::Memory32(parse_number(location)?),
            "reg" => {
                let index = location
                    .trim()
                    .trim_start_matches(['r', 'R'])
                    .parse::<usize>()
                    .map_err(|_| format!("invalid register '{location}'"))?;
                if index > 31 {
                    return Err("register index must be between r0 and r31".to_string());
                }
                Target::Register(index)
            }
            _ => return Err(format!("unknown cheat target '{kind}'")),
        };
        Ok(Self { target, value })
    }
}

fn parse_number(value: &str) -> Result<u32, String> {
    let value = value.trim();
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).map_err(|_| format!("invalid number '{value}'"))
    } else {
        value
            .parse()
            .map_err(|_| format!("invalid number '{value}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_memory_and_register_rules() {
        assert_eq!(
            "mem8:0x1234=99".parse(),
            Ok(CheatRule {
                target: Target::Memory8(0x1234),
                value: 99
            })
        );
        assert_eq!(
            "reg:r31=0x80001000".parse(),
            Ok(CheatRule {
                target: Target::Register(31),
                value: 0x80001000
            })
        );
    }

    #[test]
    fn applies_memory_and_register_rules() {
        let mut emulator = Emulator::default();
        "mem32:0x100=0x12345678"
            .parse::<CheatRule>()
            .unwrap()
            .apply(&mut emulator)
            .unwrap();
        "reg:r4=7"
            .parse::<CheatRule>()
            .unwrap()
            .apply(&mut emulator)
            .unwrap();
        assert_eq!(emulator.memory.read_u32(0x100).unwrap(), 0x12345678);
        assert_eq!(emulator.cpu.regs.read(4), 7);
    }
}
