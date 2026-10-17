//! Live counters of a running archive operation, read by the progress window.
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering::Relaxed};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Scanning,
    Compressing,
    Verifying,
    Done,
}

/// Written by the engine thread, read at any time by others; each counter is
/// individually exact, a snapshot is only approximately simultaneous.
#[derive(Debug, Default)]
pub struct Tracker {
    phase: AtomicU8,
    total_files: AtomicU64,
    total_bytes: AtomicU64,
    files_done: AtomicU64,
    bytes_read: AtomicU64,
    bytes_written: AtomicU64,
    bytes_verified: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub phase: Phase,
    /// Regular files in the manifest, including empty ones.
    pub total_files: u64,
    pub total_bytes: u64,
    pub files_done: u64,
    pub bytes_read: u64,
    /// Archive size so far, as written by the encoder.
    pub bytes_written: u64,
    pub bytes_verified: u64,
}

impl Tracker {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            phase: match self.phase.load(Relaxed) {
                1 => Phase::Compressing,
                2 => Phase::Verifying,
                3 => Phase::Done,
                _ => Phase::Scanning,
            },
            total_files: self.total_files.load(Relaxed),
            total_bytes: self.total_bytes.load(Relaxed),
            files_done: self.files_done.load(Relaxed),
            bytes_read: self.bytes_read.load(Relaxed),
            bytes_written: self.bytes_written.load(Relaxed),
            bytes_verified: self.bytes_verified.load(Relaxed),
        }
    }

    pub(crate) fn set_phase(&self, phase: Phase) {
        self.phase.store(phase as u8, Relaxed);
    }

    pub(crate) fn start(&self, files: u64, bytes: u64) {
        for counter in [
            &self.files_done,
            &self.bytes_read,
            &self.bytes_written,
            &self.bytes_verified,
        ] {
            counter.store(0, Relaxed);
        }
        self.total_files.store(files, Relaxed);
        self.total_bytes.store(bytes, Relaxed);
        self.set_phase(Phase::Compressing);
    }

    pub(crate) fn file_done(&self) {
        self.files_done.fetch_add(1, Relaxed);
    }

    pub(crate) fn read(&self, bytes: u64) {
        self.bytes_read.fetch_add(bytes, Relaxed);
    }

    pub(crate) fn written_up_to(&self, position: u64) {
        self.bytes_written.fetch_max(position, Relaxed);
    }

    pub(crate) fn verified(&self, bytes: u64) {
        self.bytes_verified.fetch_add(bytes, Relaxed);
    }
}

impl Snapshot {
    /// Overall fraction done. Verification re-reads everything, but decoding
    /// LZMA2 is far cheaper than encoding it, so it carries less weight.
    pub fn fraction(&self) -> f64 {
        const VERIFY_WEIGHT: f64 = 0.2;
        match self.phase {
            Phase::Scanning => 0.0,
            Phase::Done => 1.0,
            _ if self.total_bytes == 0 => match self.phase {
                Phase::Verifying => 1.0 / (1.0 + VERIFY_WEIGHT),
                _ => 0.0,
            },
            _ => {
                let total = self.total_bytes as f64;
                let done = self.bytes_read as f64 + VERIFY_WEIGHT * self.bytes_verified as f64;
                (done / (total * (1.0 + VERIFY_WEIGHT))).clamp(0.0, 1.0)
            }
        }
    }
}
