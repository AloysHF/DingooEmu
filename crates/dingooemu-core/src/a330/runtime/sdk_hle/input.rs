use super::super::*;

impl RuntimeBus<'_> {
    pub(super) fn dispatch_input(&mut self, cpu: &mut Cpu, name: &str) -> Result<bool> {
        match name {
            "_kbd_get_status" | "kbd_get_status" | "_rmt_get_status" | "rmt_get_status" => {
                let address = cpu.r[0];
                let (pressed, released, status) = self.input.take_status();
                let pressed = map_a330_input(self.profile, pressed);
                let released = map_a330_input(self.profile, released);
                let status = map_a330_input(self.profile, status);
                self.write_memory(address, &pressed.to_le_bytes())?;
                self.write_memory(address.wrapping_add(4), &released.to_le_bytes())?;
                self.write_memory(address.wrapping_add(8), &status.to_le_bytes())?;
                cpu.r[0] = 0;
            }
            "_kbd_get_key" | "kbd_get_key" | "_rmt_get_key" | "rmt_get_key" | "sys_get_key"
            | "KBDGetSKey" | "KBDGetSKeyStatus" | "RMTGetSKey" => {
                cpu.r[0] = map_a330_input(self.profile, self.input.buttons());
            }
            "_sys_judge_event" | "sys_judge_event" => {
                cpu.r[0] = u32::from(self.input.take_pending_event());
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
