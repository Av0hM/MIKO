//! Exclusive ownership and observable phases for local assistant audio work.
use anyhow::{Result, bail};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

pub(crate) const IDLE: u8 = 0;
pub(crate) const LISTENING: u8 = 1;
pub(crate) const TRANSCRIBING: u8 = 2;
pub(crate) const SYNTHESIZING: u8 = 3;
pub(crate) const SPEAKING: u8 = 4;

#[derive(Default)]
pub(crate) struct AudioCoordinator(AtomicU8);

impl AudioCoordinator {
    pub(crate) fn phase(&self) -> u8 {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn acquire(&self, phase: u8, timeout: Duration) -> Result<AudioGuard<'_>> {
        let start = Instant::now();
        loop {
            if self
                .0
                .compare_exchange(IDLE, phase, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(AudioGuard(self));
            }
            if start.elapsed() >= timeout {
                bail!("Audio is busy; retry when recording or playback finishes");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

pub(crate) struct AudioGuard<'a>(&'a AudioCoordinator);
impl AudioGuard<'_> {
    pub(crate) fn phase(&self, phase: u8) {
        self.0.0.store(phase, Ordering::Release);
    }
}
impl Drop for AudioGuard<'_> {
    fn drop(&mut self) {
        self.0.0.store(IDLE, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_is_exclusive_and_error_unwind_clears_phases() {
        let audio = AudioCoordinator::default();
        let failed = (|| -> Result<()> {
            let guard = audio.acquire(LISTENING, Duration::ZERO)?;
            assert_eq!(audio.phase(), LISTENING);
            assert!(audio.acquire(SPEAKING, Duration::ZERO).is_err());
            guard.phase(TRANSCRIBING);
            assert_eq!(audio.phase(), TRANSCRIBING);
            bail!("recording or transcription failed")
        })();
        assert!(failed.is_err());
        assert_eq!(audio.phase(), IDLE);
        let guard = audio.acquire(SYNTHESIZING, Duration::ZERO).unwrap();
        guard.phase(SPEAKING);
        assert_eq!(audio.phase(), SPEAKING);
        drop(guard);
        assert_eq!(audio.phase(), IDLE);
    }
}

/// Shared audio ownership across assistant and user-authored automation.
pub(crate) fn shared() -> std::sync::Arc<AudioCoordinator> {
    static AUDIO: std::sync::OnceLock<std::sync::Arc<AudioCoordinator>> =
        std::sync::OnceLock::new();
    AUDIO
        .get_or_init(|| std::sync::Arc::new(AudioCoordinator::default()))
        .clone()
}
