use anyhow::{bail, Context};
use dingooemu_core::input::{
    BUTTON_A, BUTTON_B, BUTTON_DOWN, BUTTON_L, BUTTON_LEFT, BUTTON_R, BUTTON_RIGHT, BUTTON_SELECT,
    BUTTON_START, BUTTON_UP, BUTTON_X, BUTTON_Y,
};
use dingooemu_core::video::{FramebufferStats, Video};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

const INPUT_SCRIPT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputScript {
    schema_version: u32,
    content: String,
    relative_path: String,
    content_sha256: String,
    frames: u32,
    events: Vec<InputEvent>,
    checkpoints: Vec<InputCheckpoint>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputEvent {
    frame: u32,
    buttons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputCheckpoint {
    name: String,
    frame: u32,
    expected_framebuffer_crc32: String,
    control_framebuffer_crc32: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct InputDiagnostics {
    pub schema_version: u32,
    pub content: String,
    pub relative_path: String,
    pub content_sha256: String,
    pub frames: u32,
    pub event_count: usize,
    pub nonzero_input_frames: u32,
    pub checkpoints: Vec<CheckpointDiagnostics>,
    pub all_checkpoints_passed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckpointDiagnostics {
    pub name: String,
    pub frame: u32,
    pub expected_framebuffer_crc32: String,
    pub control_framebuffer_crc32: String,
    pub actual_framebuffer_crc32: String,
    pub differs_from_control: bool,
    pub status: &'static str,
    pub framebuffer: FramebufferStats,
}

#[derive(Debug)]
pub struct InputPlayback {
    script: InputScript,
    event_masks: Vec<u32>,
    next_event: usize,
    current_buttons: u32,
    next_checkpoint: usize,
    diagnostics: InputDiagnostics,
}

impl InputPlayback {
    pub fn load(path: &Path, content_name: &str, requested_frames: u32) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read input script {}", path.display()))?;
        let script: InputScript = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse input script {}", path.display()))?;
        Self::new(script, content_name, requested_frames)
    }

    fn new(script: InputScript, content_name: &str, requested_frames: u32) -> anyhow::Result<Self> {
        if script.schema_version != INPUT_SCRIPT_SCHEMA_VERSION {
            bail!(
                "unsupported input script schema version {}; expected {}",
                script.schema_version,
                INPUT_SCRIPT_SCHEMA_VERSION
            );
        }
        if script.content != content_name {
            bail!(
                "input script content '{}' does not match '{}'",
                script.content,
                content_name
            );
        }
        if script.frames != requested_frames {
            bail!(
                "input script requires {} frames, but the run requests {}",
                script.frames,
                requested_frames
            );
        }
        validate_sha256(&script.content_sha256)?;
        if script.relative_path.is_empty() || script.relative_path.contains('\\') {
            bail!("input script relative_path must use a non-empty forward-slash path");
        }
        if script.events.is_empty() {
            bail!("input script must contain at least one input event");
        }
        if script.checkpoints.is_empty() {
            bail!("input script must contain at least one checkpoint");
        }

        let mut event_masks = Vec::with_capacity(script.events.len());
        let mut previous_event_frame = None;
        for event in &script.events {
            if event.frame >= script.frames {
                bail!("input event frame {} is outside the run", event.frame);
            }
            if previous_event_frame.is_some_and(|previous| event.frame <= previous) {
                bail!("input event frames must be strictly increasing");
            }
            previous_event_frame = Some(event.frame);
            event_masks.push(parse_buttons(&event.buttons)?);
        }

        let mut checkpoint_names = HashSet::new();
        let mut previous_checkpoint_frame = None;
        for checkpoint in &script.checkpoints {
            if checkpoint.name.trim().is_empty() || !checkpoint_names.insert(&checkpoint.name) {
                bail!("input checkpoint names must be non-empty and unique");
            }
            if checkpoint.frame == 0 || checkpoint.frame > script.frames {
                bail!(
                    "input checkpoint frame {} is outside the completed-frame range",
                    checkpoint.frame
                );
            }
            if previous_checkpoint_frame.is_some_and(|previous| checkpoint.frame <= previous) {
                bail!("input checkpoint frames must be strictly increasing");
            }
            previous_checkpoint_frame = Some(checkpoint.frame);
            validate_crc32(&checkpoint.expected_framebuffer_crc32)?;
            validate_crc32(&checkpoint.control_framebuffer_crc32)?;
            if checkpoint
                .expected_framebuffer_crc32
                .eq_ignore_ascii_case(&checkpoint.control_framebuffer_crc32)
            {
                bail!("input checkpoint expected CRC32 must differ from its no-input control");
            }
        }

        let diagnostics = InputDiagnostics {
            schema_version: script.schema_version,
            content: script.content.clone(),
            relative_path: script.relative_path.clone(),
            content_sha256: script.content_sha256.clone(),
            frames: script.frames,
            event_count: script.events.len(),
            nonzero_input_frames: 0,
            checkpoints: Vec::with_capacity(script.checkpoints.len()),
            all_checkpoints_passed: false,
        };

        Ok(Self {
            script,
            event_masks,
            next_event: 0,
            current_buttons: 0,
            next_checkpoint: 0,
            diagnostics,
        })
    }

    pub fn buttons_for_frame(&mut self, frame: u32) -> u32 {
        if self
            .script
            .events
            .get(self.next_event)
            .is_some_and(|event| event.frame == frame)
        {
            self.current_buttons = self.event_masks[self.next_event];
            self.next_event += 1;
        }
        if self.current_buttons != 0 {
            self.diagnostics.nonzero_input_frames += 1;
        }
        self.current_buttons
    }

    pub fn record_checkpoint(&mut self, completed_frame: u32, video: &Video) {
        let Some(checkpoint) = self.script.checkpoints.get(self.next_checkpoint) else {
            return;
        };
        if checkpoint.frame != completed_frame {
            return;
        }

        let actual = format!("{:08x}", video.framebuffer_crc32());
        let differs_from_control =
            !actual.eq_ignore_ascii_case(&checkpoint.control_framebuffer_crc32);
        let passed = actual.eq_ignore_ascii_case(&checkpoint.expected_framebuffer_crc32)
            && differs_from_control;
        self.diagnostics.checkpoints.push(CheckpointDiagnostics {
            name: checkpoint.name.clone(),
            frame: checkpoint.frame,
            expected_framebuffer_crc32: checkpoint.expected_framebuffer_crc32.to_ascii_lowercase(),
            control_framebuffer_crc32: checkpoint.control_framebuffer_crc32.to_ascii_lowercase(),
            actual_framebuffer_crc32: actual,
            differs_from_control,
            status: if passed { "pass" } else { "fail" },
            framebuffer: video.framebuffer_stats(),
        });
        self.next_checkpoint += 1;
        self.diagnostics.all_checkpoints_passed = self.next_checkpoint
            == self.script.checkpoints.len()
            && self
                .diagnostics
                .checkpoints
                .iter()
                .all(|checkpoint| checkpoint.status == "pass");
    }

    pub fn diagnostics(&self) -> &InputDiagnostics {
        &self.diagnostics
    }
}

fn parse_buttons(buttons: &[String]) -> anyhow::Result<u32> {
    let mut mask = 0;
    for button in buttons {
        let value = match button.to_ascii_lowercase().as_str() {
            "up" => BUTTON_UP,
            "down" => BUTTON_DOWN,
            "left" => BUTTON_LEFT,
            "right" => BUTTON_RIGHT,
            "a" => BUTTON_A,
            "b" => BUTTON_B,
            "x" => BUTTON_X,
            "y" => BUTTON_Y,
            "start" => BUTTON_START,
            "select" => BUTTON_SELECT,
            "l" => BUTTON_L,
            "r" => BUTTON_R,
            _ => bail!("unknown input script button '{button}'"),
        };
        if mask & value != 0 {
            bail!("duplicate input script button '{button}'");
        }
        mask |= value;
    }
    Ok(mask)
}

fn validate_sha256(value: &str) -> anyhow::Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("content_sha256 must contain exactly 64 hexadecimal characters");
    }
    Ok(())
}

fn validate_crc32(value: &str) -> anyhow::Result<()> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("framebuffer CRC32 values must contain exactly 8 hexadecimal characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script() -> InputScript {
        InputScript {
            schema_version: 1,
            content: "game.app".to_string(),
            relative_path: "game.app".to_string(),
            content_sha256: "00".repeat(32),
            frames: 4,
            events: vec![
                InputEvent {
                    frame: 1,
                    buttons: vec!["a".to_string(), "right".to_string()],
                },
                InputEvent {
                    frame: 3,
                    buttons: vec![],
                },
            ],
            checkpoints: vec![InputCheckpoint {
                name: "menu-moved".to_string(),
                frame: 4,
                expected_framebuffer_crc32: "066e64a1".to_string(),
                control_framebuffer_crc32: "ffffffff".to_string(),
            }],
        }
    }

    #[test]
    fn playback_applies_state_events_and_records_checkpoint() {
        let mut playback = InputPlayback::new(script(), "game.app", 4).unwrap();
        assert_eq!(playback.buttons_for_frame(0), 0);
        assert_eq!(playback.buttons_for_frame(1), BUTTON_A | BUTTON_RIGHT);
        assert_eq!(playback.buttons_for_frame(2), BUTTON_A | BUTTON_RIGHT);
        assert_eq!(playback.buttons_for_frame(3), 0);

        playback.record_checkpoint(4, &Video::new());
        assert_eq!(playback.diagnostics.nonzero_input_frames, 2);
        assert_eq!(playback.diagnostics.checkpoints.len(), 1);
        assert!(playback.diagnostics.all_checkpoints_passed);
    }

    #[test]
    fn playback_rejects_wrong_content_and_invalid_ordering() {
        assert!(InputPlayback::new(script(), "other.app", 4).is_err());

        let mut unordered = script();
        unordered.events[1].frame = unordered.events[0].frame;
        assert!(InputPlayback::new(unordered, "game.app", 4).is_err());
    }
}
