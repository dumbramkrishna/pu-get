//! Progress snapshots and speed/ETA calculation.

use std::time::{Duration, Instant};

/// A point-in-time progress sample emitted by the download manager.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProgressSnapshot {
    /// Bytes present in the `.part` file (including resumed bytes).
    pub downloaded: u64,
    /// Expected total size, when the server or catalog knows it.
    pub total: Option<u64>,
    /// Bytes transferred per second during this session.
    pub bytes_per_second: f64,
    /// Estimated time remaining, when `total` and `bytes_per_second` are known.
    pub eta: Option<Duration>,
}

impl ProgressSnapshot {
    /// Completion in `0.0..=1.0`. Returns `0.0` when the total is unknown.
    #[must_use]
    pub fn fraction(self) -> f64 {
        match self.total {
            Some(total) if total > 0 => (self.downloaded as f64 / total as f64).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }
}

/// Mutable byte counter with wall-clock speed tracking.
#[derive(Debug)]
pub struct ProgressTracker {
    start_offset: u64,
    downloaded: u64,
    total: Option<u64>,
    started_at: Instant,
}

impl ProgressTracker {
    /// Create a tracker. `initial` is the number of bytes already on disk when
    /// the job starts, so resumed downloads do not get inflated speeds.
    #[must_use]
    pub fn new(initial: u64, total: Option<u64>) -> Self {
        Self {
            start_offset: initial,
            downloaded: initial,
            total,
            started_at: Instant::now(),
        }
    }

    /// Update the number of bytes on disk (for example after re-checking a
    /// `.part` file between retries).
    pub fn set_position(&mut self, downloaded: u64) {
        self.downloaded = downloaded;
    }

    /// Update the expected total size once a response supplies one.
    pub fn set_total(&mut self, total: Option<u64>) {
        if total.is_some() {
            self.total = total;
        }
    }

    /// Record freshly received bytes.
    pub fn record(&mut self, bytes: u64) {
        self.downloaded = self.downloaded.saturating_add(bytes);
    }

    /// Bytes on disk.
    #[must_use]
    pub fn downloaded(&self) -> u64 {
        self.downloaded
    }

    /// Expected total size.
    #[must_use]
    pub fn total(&self) -> Option<u64> {
        self.total
    }

    /// Take a snapshot with current speed and ETA.
    #[must_use]
    pub fn snapshot(&self) -> ProgressSnapshot {
        let elapsed = self.started_at.elapsed().as_secs_f64().max(0.001);
        let transferred = self.downloaded.saturating_sub(self.start_offset);
        let bytes_per_second = transferred as f64 / elapsed;

        let eta = match self.total {
            Some(total) if total <= self.downloaded => Some(Duration::ZERO),
            Some(total) if bytes_per_second > 0.0 => {
                let remaining = total.saturating_sub(self.downloaded) as f64;
                let seconds = (remaining / bytes_per_second).min(7.0 * 86_400.0);
                Some(Duration::from_secs_f64(seconds))
            }
            _ => None,
        };

        ProgressSnapshot {
            downloaded: self.downloaded,
            total: self.total,
            bytes_per_second,
            eta,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn fraction_is_clamped_and_handles_unknown_totals() {
        let snapshot = ProgressSnapshot {
            downloaded: 50,
            total: Some(200),
            ..ProgressSnapshot::default()
        };
        assert!((snapshot.fraction() - 0.25).abs() < f64::EPSILON);

        let complete = ProgressSnapshot {
            downloaded: 200,
            total: Some(200),
            ..ProgressSnapshot::default()
        };
        assert!((complete.fraction() - 1.0).abs() < f64::EPSILON);

        let unknown = ProgressSnapshot {
            downloaded: 10,
            total: None,
            ..ProgressSnapshot::default()
        };
        assert!((unknown.fraction() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn tracker_accounts_for_resumed_bytes_in_speed() {
        let mut tracker = ProgressTracker::new(100, Some(1_000));
        thread::sleep(Duration::from_millis(5));
        tracker.record(100);

        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.downloaded, 200);
        assert_eq!(snapshot.total, Some(1_000));
        assert!(snapshot.bytes_per_second > 0.0);
        assert!(snapshot.eta.is_some());
    }

    #[test]
    fn tracker_reports_zero_eta_when_complete() {
        let tracker = ProgressTracker::new(500, Some(500));
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.eta, Some(Duration::ZERO));
    }

    #[test]
    fn tracker_without_total_has_no_eta() {
        let mut tracker = ProgressTracker::new(0, None);
        tracker.record(42);
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.total, None);
        assert!(snapshot.eta.is_none());
    }
}
