use crate::common::cheats::{CheatManager, CheatParseError, CheatRule, MemoryWidth};

use super::{cpu::Cpu, memory::Memory};

pub(super) fn set_slot(
    manager: &mut CheatManager,
    index: u32,
    enabled: bool,
    code: &str,
    memory: &Memory,
) -> Result<(), CheatParseError> {
    manager.set_slot_with_validator(index, enabled, code, |rule| validate(rule, memory))
}

pub(super) fn set_parsed_rule(
    manager: &mut CheatManager,
    index: u32,
    enabled: bool,
    rule: CheatRule,
    memory: &Memory,
) -> Result<(), CheatParseError> {
    manager.set_parsed_rule_with_validator(index, enabled, rule, |rule| validate(rule, memory))
}

pub(super) fn apply(manager: &CheatManager, memory: &mut Memory, cpu: &mut Cpu) {
    for rule in manager.enabled_rules() {
        let result = match *rule {
            CheatRule::Memory {
                width,
                address,
                value,
            } => match width {
                MemoryWidth::U8 => memory.write8(address, value as u8),
                MemoryWidth::U16 => memory.write16(address, value as u16),
                MemoryWidth::U32 => memory.write32(address, value),
            },
            CheatRule::Register { index, value } => {
                cpu.r[index] = value;
                Ok(())
            }
        };
        if let Err(error) = result {
            log::warn!("Failed to apply A330 cheat: {error}");
        }
    }
}

fn validate(rule: &CheatRule, memory: &Memory) -> Result<(), CheatParseError> {
    match rule {
        CheatRule::Memory { width, address, .. } => {
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
        }
        CheatRule::Register { index, .. } if *index >= 16 => {
            return Err(CheatParseError::InvalidRegister(index.to_string()));
        }
        CheatRule::Register { .. } => {}
    }
    Ok(())
}
