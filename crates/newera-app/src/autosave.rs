//! When a tab sends its cloud project back to the service: a moment after the
//! last change, one save at a time, and whatever changed meanwhile in the next
//! one. Only the timing lives here; the request is `ai_web`'s.

/// How long the document stays unchanged before it is sent: the same breath
/// the browser's own mirror waits (`NewEraApp::mirror`), so a run of edits
/// goes as one save.
pub(crate) const QUIET_MS: f64 = 1500.0;

/// How long a save that failed (no network, the plan full) waits before the
/// next try, as the mirror does after a refused write.
pub(crate) const RETRY_MS: f64 = 30_000.0;

/// What has been sent, what is on its way, and the revision the service holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Autosave {
    /// The service's revision of the project, as it last said.
    server: i64,
    /// The document revision the service holds.
    saved: u64,
    /// The document revision last seen, and when it was first seen.
    seen: (u64, f64),
    /// The document revision on its way, while a save is out.
    sending: Option<u64>,
    /// When the last save failed: the next try waits from then. A save
    /// only goes once that wait is over, so one that went through needs no
    /// clearing of it.
    failed_at: Option<f64>,
    /// A save was refused as stale: nothing goes until the service's copy is
    /// open in place of this one, or the attempt to get it has failed.
    reloading: bool,
    /// The document revision last put down as a backup, so a retry after a
    /// failed reload does not download the same state again.
    backed_up: Option<u64>,
}

impl Autosave {
    /// A project just loaded at the service's `server` revision, which is the
    /// document's `revision` now.
    pub(crate) fn new(server: i64, revision: u64, now: f64) -> Self {
        Self {
            server,
            saved: revision,
            seen: (revision, now),
            sending: None,
            failed_at: None,
            reloading: false,
            backed_up: None,
        }
    }

    /// Whether the document, at `revision`, is to be sent now; if so, the
    /// service revision to send it on, and it counts as on its way.
    pub(crate) fn due(&mut self, revision: u64, now: f64) -> Option<i64> {
        if revision != self.seen.0 {
            self.seen = (revision, now);
        }
        if self.reloading || self.sending.is_some() || revision == self.saved {
            return None;
        }
        if now - self.seen.1 < QUIET_MS {
            return None;
        }
        if self.failed_at.is_some_and(|at| now - at < RETRY_MS) {
            return None;
        }
        self.sending = Some(revision);
        Some(self.server)
    }

    /// The save on its way was kept, as the service's `server` revision.
    pub(crate) fn saved(&mut self, server: i64) {
        if let Some(sent) = self.sending.take() {
            self.saved = sent;
        }
        self.server = server;
    }

    /// The save on its way did not go through; the next try waits.
    pub(crate) fn failed(&mut self, now: f64) {
        self.sending = None;
        self.failed_at = Some(now);
    }

    /// The save on its way was refused as stale: nothing more goes while the
    /// service's copy is fetched.
    pub(crate) fn conflicted(&mut self) {
        self.sending = None;
        self.reloading = true;
    }

    /// Whether the document, at `revision`, is yet to be put down as a
    /// backup before the service's copy replaces it.
    pub(crate) fn needs_backup(&self, revision: u64) -> bool {
        self.backed_up != Some(revision)
    }

    /// The document at `revision` is down as a backup.
    pub(crate) fn backed_up(&mut self, revision: u64) {
        self.backed_up = Some(revision);
    }

    /// The service's copy did not come (or the backup did not go down): the
    /// next try is the normal retry, from now, and saves go on until then.
    pub(crate) fn reload_failed(&mut self, now: f64) {
        self.reloading = false;
        self.failed_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::{Autosave, QUIET_MS, RETRY_MS};

    #[test]
    fn nothing_goes_while_nothing_changed() {
        let mut save = Autosave::new(7, 3, 0.0);
        assert_eq!(save.due(3, 0.0), None);
        assert_eq!(save.due(3, 10.0 * RETRY_MS), None);
    }

    #[test]
    fn a_change_goes_once_the_document_has_been_quiet() {
        let mut save = Autosave::new(7, 3, 0.0);
        assert_eq!(save.due(4, 100.0), None, "just changed");
        assert_eq!(save.due(5, 100.0 + QUIET_MS - 1.0), None, "changed again");
        assert_eq!(
            save.due(5, 100.0 + QUIET_MS),
            None,
            "quiet since the second change only"
        );
        assert_eq!(save.due(5, 100.0 + 2.0 * QUIET_MS - 1.0), Some(7));
    }

    #[test]
    fn one_save_at_a_time_and_the_latest_state_next() {
        let mut save = Autosave::new(7, 3, 0.0);
        save.due(4, 0.0);
        assert_eq!(save.due(4, QUIET_MS), Some(7));
        // Changed while the first is out: it waits for the answer.
        save.due(6, QUIET_MS + 1.0);
        assert_eq!(save.due(6, 10.0 * QUIET_MS), None);
        save.saved(8);
        // The answer's revision is the one the next save goes on.
        assert_eq!(save.due(6, 10.0 * QUIET_MS), Some(8));
        save.saved(9);
        assert_eq!(save.due(6, 20.0 * QUIET_MS), None, "all of it is kept");
        assert_eq!(
            save,
            Autosave {
                server: 9,
                saved: 6,
                seen: (6, QUIET_MS + 1.0),
                sending: None,
                failed_at: None,
                reloading: false,
                backed_up: None,
            }
        );
    }

    #[test]
    fn a_failed_save_is_tried_again_later_on_the_same_revision() {
        let mut save = Autosave::new(7, 3, 0.0);
        save.due(4, 0.0);
        assert_eq!(save.due(4, QUIET_MS), Some(7));
        save.failed(QUIET_MS);
        assert_eq!(save.due(4, QUIET_MS + RETRY_MS - 1.0), None);
        assert_eq!(save.due(4, QUIET_MS + RETRY_MS), Some(7));
        save.saved(8);
        // Once one went through, the next change waits only for quiet.
        save.due(5, 100.0 * QUIET_MS);
        assert_eq!(save.due(5, 101.0 * QUIET_MS), Some(8));
    }

    #[test]
    fn a_stale_save_stops_the_saves_until_the_reload_is_over() {
        let mut save = Autosave::new(7, 3, 0.0);
        save.due(4, 0.0);
        assert_eq!(save.due(4, QUIET_MS), Some(7));
        save.conflicted();
        assert_eq!(save.due(5, 100.0 * QUIET_MS), None);
        assert_eq!(save.due(5, 200.0 * QUIET_MS), None, "still fetching");
    }

    #[test]
    fn a_failed_reload_goes_back_to_the_normal_retry_and_never_stays_stuck() {
        let mut save = Autosave::new(7, 3, 0.0);
        save.due(4, 0.0);
        assert_eq!(save.due(4, QUIET_MS), Some(7));
        save.conflicted();
        save.reload_failed(QUIET_MS + 10.0);
        assert_eq!(save.due(4, QUIET_MS + 10.0 + RETRY_MS - 1.0), None);
        assert_eq!(
            save.due(4, QUIET_MS + 10.0 + RETRY_MS),
            Some(7),
            "the next attempt follows 30 s after the failure"
        );
        // Refused again: one more wait, not a loop.
        save.conflicted();
        save.reload_failed(100.0 * QUIET_MS);
        assert_eq!(save.due(4, 100.0 * QUIET_MS + 1.0), None);
        assert_eq!(save.due(4, 100.0 * QUIET_MS + RETRY_MS), Some(7));
    }

    #[test]
    fn the_same_state_is_put_down_as_a_backup_once() {
        let mut save = Autosave::new(7, 3, 0.0);
        assert!(save.needs_backup(4));
        save.backed_up(4);
        assert!(!save.needs_backup(4), "a retry does not download it again");
        assert!(save.needs_backup(5), "an edit since is backed up too");
    }
}
