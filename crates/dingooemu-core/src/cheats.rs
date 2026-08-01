use std::collections::BTreeMap;
use std::str::FromStr;

use thiserror::Error;

use crate::cpu::Cpu;
use crate::memory::Memory;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CheatParseError {
    #[error("cheat code is empty")]
    Empty,
    #[error("cheat code must use '<target>=<value>' syntax")]
    MissingValue,
    #[error("unknown cheat target '{0}'")]
    UnknownTarget(String),
    #[error("invalid numeric value '{0}'")]
    InvalidNumber(String),
    #[error("{0}-bit cheat value is out of range")]
    ValueOutOfRange(u32),
    #[error("{0}-bit memory address 0x{1:08X} is not aligned")]
    MisalignedAddress(u32, u32),
    #[error("memory range 0x{address:08X}..0x{end:08X} is not writable RAM or framebuffer")]
    InvalidMemoryRange { address: u32, end: u32 },
    #[error("unknown MIPS register '{0}'")]
    InvalidRegister(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryWidth {
    U8,
    U16,
    U32,
}

impl MemoryWidth {
    const fn bytes(self) -> u32 {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::U32 => 4,
        }
    }

    const fn bits(self) -> u32 {
        self.bytes() * 8
    }

    const fn max_value(self) -> u64 {
        match self {
            Self::U8 => u8::MAX as u64,
            Self::U16 => u16::MAX as u64,
            Self::U32 => u32::MAX as u64,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheatRule {
    Memory {
        width: MemoryWidth,
        address: u32,
        value: u32,
    },
    Register {
        index: usize,
        value: u32,
    },
}

impl CheatRule {
    fn validate(&self, memory: &Memory) -> Result<(), CheatParseError> {
        let Self::Memory { width, address, .. } = self else {
            return Ok(());
        };
        let bytes = width.bytes();
        if address % bytes != 0 {
            return Err(CheatParseError::MisalignedAddress(width.bits(), *address));
        }
        if !memory.is_cheat_writable_range(*address, bytes as usize) {
            return Err(CheatParseError::InvalidMemoryRange {
                address: *address,
                end: address.saturating_add(bytes - 1),
            });
        }
        Ok(())
    }

    fn apply(&self, memory: &mut Memory, cpu: &mut Cpu) {
        let result = match *self {
            Self::Memory {
                width,
                address,
                value,
            } => match width {
                MemoryWidth::U8 => memory.write_u8(address, value as u8),
                MemoryWidth::U16 => memory.write_u16(address, value as u16),
                MemoryWidth::U32 => memory.write_u32(address, value),
            },
            Self::Register { index, value } => {
                cpu.regs.write(index, value);
                Ok(())
            }
        };
        if let Err(error) = result {
            log::warn!("Failed to apply cheat: {error}");
        }
    }
}

impl FromStr for CheatRule {
    type Err = CheatParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.is_empty() {
            return Err(CheatParseError::Empty);
        }
        let (target, value) = input.split_once('=').ok_or(CheatParseError::MissingValue)?;
        let target = target.trim();
        let value = parse_number(value)?;
        let target_lower = target.to_ascii_lowercase();

        for (prefix, width) in [
            ("mem8:", MemoryWidth::U8),
            ("mem16:", MemoryWidth::U16),
            ("mem32:", MemoryWidth::U32),
        ] {
            if let Some(address) = target_lower.strip_prefix(prefix) {
                if value > width.max_value() {
                    return Err(CheatParseError::ValueOutOfRange(width.bits()));
                }
                return Ok(Self::Memory {
                    width,
                    address: parse_u32(address)?,
                    value: value as u32,
                });
            }
        }

        if let Some(register) = target_lower.strip_prefix("reg:") {
            let register = register.trim().trim_start_matches('r');
            let index = register
                .parse::<usize>()
                .ok()
                .filter(|index| *index <= 31)
                .ok_or_else(|| CheatParseError::InvalidRegister(register.to_string()))?;
            return Ok(Self::Register {
                index,
                value: u32::try_from(value).map_err(|_| CheatParseError::ValueOutOfRange(32))?,
            });
        }

        Err(CheatParseError::UnknownTarget(target.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheatSlot {
    pub enabled: bool,
    pub code: String,
    pub rule: CheatRule,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheatManager {
    slots: BTreeMap<u32, CheatSlot>,
}

impl CheatManager {
    pub fn clear(&mut self) {
        self.slots.clear();
    }

    pub fn set_slot(
        &mut self,
        index: u32,
        enabled: bool,
        code: &str,
        memory: &Memory,
    ) -> Result<(), CheatParseError> {
        let code = code.trim();
        if code.is_empty() {
            self.slots.remove(&index);
            return Ok(());
        }
        let rule = CheatRule::from_str(code)?;
        self.set_rule(index, enabled, code.to_string(), rule, memory)
    }

    pub fn set_parsed_rule(
        &mut self,
        index: u32,
        enabled: bool,
        rule: CheatRule,
        memory: &Memory,
    ) -> Result<(), CheatParseError> {
        self.set_rule(index, enabled, String::new(), rule, memory)
    }

    fn set_rule(
        &mut self,
        index: u32,
        enabled: bool,
        code: String,
        rule: CheatRule,
        memory: &Memory,
    ) -> Result<(), CheatParseError> {
        rule.validate(memory)?;
        self.slots.insert(
            index,
            CheatSlot {
                enabled,
                code,
                rule,
            },
        );
        Ok(())
    }

    pub fn get_slot(&self, index: u32) -> Option<&CheatSlot> {
        self.slots.get(&index)
    }

    pub fn apply(&self, memory: &mut Memory, cpu: &mut Cpu) {
        for slot in self.slots.values().filter(|slot| slot.enabled) {
            slot.rule.apply(memory, cpu);
        }
    }
}

fn parse_number(input: &str) -> Result<u64, CheatParseError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(CheatParseError::InvalidNumber(input.to_string()));
    }
    input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
        .map_or_else(|| input.parse::<u64>(), |hex| u64::from_str_radix(hex, 16))
        .map_err(|_| CheatParseError::InvalidNumber(input.to_string()))
}

fn parse_u32(input: &str) -> Result<u32, CheatParseError> {
    u32::try_from(parse_number(input)?)
        .map_err(|_| CheatParseError::InvalidNumber(input.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates_rules() {
        assert_eq!(
            "mem16:0x1234=99".parse(),
            Ok(CheatRule::Memory {
                width: MemoryWidth::U16,
                address: 0x1234,
                value: 99,
            })
        );
        assert_eq!(
            "reg:r31=0x80001000".parse(),
            Ok(CheatRule::Register {
                index: 31,
                value: 0x8000_1000,
            })
        );
        assert!(matches!(
            "mem8:0x100=256".parse::<CheatRule>(),
            Err(CheatParseError::ValueOutOfRange(8))
        ));
    }

    #[test]
    fn applies_only_enabled_slots() {
        let mut memory = Memory::new();
        let mut cpu = Cpu::new(0);
        let mut cheats = CheatManager::default();
        cheats
            .set_slot(0, true, "mem32:0x100=0x12345678", &memory)
            .unwrap();
        cheats.set_slot(1, false, "reg:r4=7", &memory).unwrap();
        cheats.apply(&mut memory, &mut cpu);
        assert_eq!(memory.read_u32(0x100).unwrap(), 0x1234_5678);
        assert_eq!(cpu.regs.read(4), 0);
    }
}
