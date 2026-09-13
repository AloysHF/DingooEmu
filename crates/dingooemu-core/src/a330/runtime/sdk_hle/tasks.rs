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
                    self.tasks.push_back(GuestTask::new(task, cpu.r[3] & 0xff));
                }
                cpu.r[0] = 0;
            }
            "OSTaskQuery" => {
                let priority = cpu.r[0] & 0xff;
                cpu.r[0] = if priority == self.current_priority
                    || self.tasks.iter().any(|task| task.priority == priority)
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
                    self.tasks.iter().position(|task| task.priority == priority)
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
                            self.write_memory(cpu.r[2], &[0])?;
                        }
                        cpu.r[0] = 0;
                    }
                    Some(_) if self.profile == ArmProfile::Retail => {
                        cpu.r[15] = cpu.r[15].wrapping_sub(4);
                        self.requested_delay_ticks = 1;
                        self.yield_requested = true;
                    }
                    Some(_) => {
                        self.yield_requested = true;
                        cpu.r[0] = 0;
                    }
                    None => {
                        if cpu.r[2] != 0 {
                            self.write_memory(cpu.r[2], &[4])?;
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
            "OSTimeDly" | "delay" => {
                let ticks = cpu.r[0];
                cpu.r[0] = 0;
                if ticks == 1 && *self.current_audio_producer {
                    // Audio pacing yield only. The guest is about to refill the
                    // device buffer; parking here for a full OS tick starves
                    // the host queue and causes underruns. Supplemental rounds
                    // let this task write again within the same host frame.
                    self.sleep_requested = true;
                } else {
                    // OSTimeDly/delay take uC/OS-II ticks (10 ms each).
                    self.requested_delay_ticks = ticks.max(1);
                    self.sleep_requested = true;
                }
            }
            "delay_ms" => {
                let milliseconds = cpu.r[0];
                cpu.r[0] = 0;
                let ticks = milliseconds_to_os_ticks(milliseconds as u64);
                self.requested_delay_ticks = ticks;
                self.sleep_requested = true;
            }
            "OSTimeDlyHMSM" => {
                let hours = cpu.r[0];
                let minutes = cpu.r[1];
                let seconds = cpu.r[2];
                let milliseconds = cpu.r[3];
                cpu.r[0] = 0;
                let total_ms = (u64::from(hours) * 3_600_000)
                    + (u64::from(minutes) * 60_000)
                    + (u64::from(seconds) * 1_000)
                    + u64::from(milliseconds);
                self.requested_delay_ticks = milliseconds_to_os_ticks(total_ms);
                self.sleep_requested = true;
            }
            "OSTimeGet" => cpu.r[0] = self.guest_os_tick as u32,
            "GetTickCount" => cpu.r[0] = (self.guest_micros / 1_000) as u32,
            "OSTimerGetTickTimeus" => cpu.r[0] = self.guest_micros as u32,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

fn milliseconds_to_os_ticks(milliseconds: u64) -> u32 {
    let ticks = milliseconds
        .saturating_mul(OS_TICKS_PER_SECOND)
        .div_ceil(1_000);
    ticks.clamp(1, u32::MAX as u64) as u32
}
