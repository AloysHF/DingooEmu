use super::super::*;

impl RuntimeBus<'_> {
    pub(super) fn dispatch_tasks(&mut self, cpu: &mut Cpu, name: &str) -> Result<bool> {
        match name {
            "vxGoHome" | "abort" | "av_end_thread" | "av_queue_abort" => {
                self.stop_requested = true;
            }
            "OSTaskCreate" => {
                if cpu.r[0] != 0 && cpu.r[2] != 0 {
                    let mut task = Cpu::new(cpu.r[0], cpu.r[2], EXIT_ADDRESS);
                    task.set_unknown_instruction_policy(cpu.unknown_instruction_policy());
                    task.r[0] = cpu.r[1];
                    task.start();
                    self.tasks.push_back((task, cpu.r[3] & 0xff));
                }
                cpu.r[0] = 0;
            }
            "OSTaskQuery" => {
                let priority = cpu.r[0] & 0xff;
                cpu.r[0] = if priority == self.current_priority
                    || self.tasks.iter().any(|(_, value)| *value == priority)
                {
                    0
                } else {
                    41
                };
            }
            "OSTaskDel" => {
                let priority = cpu.r[0] & 0xff;
                if priority == self.current_priority {
                    self.finish_current = true;
                    cpu.r[0] = 0;
                } else if let Some(index) =
                    self.tasks.iter().position(|(_, value)| *value == priority)
                {
                    self.tasks.remove(index);
                    cpu.r[0] = 0;
                } else {
                    cpu.r[0] = 41;
                }
            }
            "OSSemCreate" => {
                let initial = cpu.r[0];
                let handle = self.allocate(16);
                if handle != 0 {
                    self.semaphores.insert(handle, initial);
                }
                cpu.r[0] = handle;
            }
            "OSSemPend" => {
                let handle = cpu.r[0];
                match self.semaphores.get_mut(&handle) {
                    Some(count) if *count > 0 => {
                        *count -= 1;
                        if cpu.r[2] != 0 {
                            self.memory.write8(cpu.r[2], 0)?;
                        }
                        cpu.r[0] = 0;
                    }
                    Some(_) if self.profile == ArmProfile::Retail => {
                        cpu.r[15] = cpu.r[15].wrapping_sub(4);
                        self.yield_requested = true;
                    }
                    Some(_) => {
                        self.yield_requested = true;
                        cpu.r[0] = 0;
                    }
                    None => {
                        if cpu.r[2] != 0 {
                            self.memory.write8(cpu.r[2], 4)?;
                        }
                        cpu.r[0] = 0;
                    }
                }
            }
            "OSSemPost" => {
                cpu.r[0] = match self.semaphores.get_mut(&cpu.r[0]) {
                    Some(count) => {
                        *count = count.saturating_add(1);
                        0
                    }
                    None => 41,
                };
            }
            "OSSemDel" => {
                cpu.r[0] = if self.semaphores.remove(&cpu.r[0]).is_some() {
                    0
                } else {
                    41
                };
            }
            "OSTimeDly" | "delay" | "delay_ms" | "OSTimeDlyHMSM" => {
                cpu.r[0] = 0;
                self.yield_requested = true;
            }
            "OSTimeGet" => cpu.r[0] = (cpu.instruction_count / 150_000) as u32,
            "GetTickCount" => cpu.r[0] = (cpu.instruction_count / 15_000) as u32,
            "OSTimerGetTickTimeus" => cpu.r[0] = (cpu.instruction_count / 15) as u32,
            _ => return Ok(false),
        }
        Ok(true)
    }
}
