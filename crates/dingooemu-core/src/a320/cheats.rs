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
                MemoryWidth::U8 => memory.write_u8(address, value as u8),
                MemoryWidth::U16 => memory.write_u16(address, value as u16),
                MemoryWidth::U32 => memory.write_u32(address, value),
            },
            CheatRule::Register { index, value } => {
                cpu.regs.write(index, value);
                Ok(())
            }
        };
        if let Err(error) = result {
            log::warn!("Failed to apply A320 cheat: {error}");
        }
    }
}

fn validate(rule: &CheatRule, memory: &Memory) -> Result<(), CheatParseError> {
    let CheatRule::Memory { width, address, .. } = rule else {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_only_enabled_slots() {
        let mut memory = Memory::new();
        let mut cpu = Cpu::new(0);
        let mut manager = CheatManager::default();
        set_slot(&mut manager, 0, true, "mem32:0x100=0x12345678", &memory).unwrap();
        set_slot(&mut manager, 1, false, "reg:r4=7", &memory).unwrap();

        apply(&manager, &mut memory, &mut cpu);

        assert_eq!(memory.read_u32(0x100).unwrap(), 0x1234_5678);
        assert_eq!(cpu.regs.read(4), 0);
    }
}
