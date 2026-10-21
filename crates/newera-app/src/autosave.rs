//! When a tab sends its cloud project back to the service: a moment after the
//! last change, one save at a time, and whatever changed meanwhile in the next
//! one — or at once, when the service asks before a render (`flush`, and its
//! answer, `flush_answer`). The timing and what is said lives here; the
//! requests are `ai_web`'s.

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
    /// A file of the project was replaced (a product's model the service
    /// sent) since the save the service holds: the document is to be sent
    /// although its revision did not move.
    files_changed: bool,
    /// The save on its way carries such a file: a try that fails leaves it
    /// to send again.
    sending_files: bool,
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
            files_changed: false,
            sending_files: false,
        }
    }

    /// A file of the project was replaced with no change to the document:
    /// it is saved as a change is.
    pub(crate) fn files_changed(&mut self) {
        self.files_changed = true;
    }

    /// Whether the service holds the document as it is at `revision`.
    fn held(&self, revision: u64) -> bool {
        revision == self.saved && !self.files_changed
    }

    /// The document at `revision` is on its way.
    fn send(&mut self, revision: u64) {
        self.sending = Some(revision);
        self.sending_files = std::mem::take(&mut self.files_changed);
    }

    /// Whether the document, at `revision`, is to be sent now; if so, the
    /// service revision to send it on, and it counts as on its way.
    pub(crate) fn due(&mut self, revision: u64, now: f64) -> Option<i64> {
        if revision != self.seen.0 {
            self.seen = (revision, now);
        }
        if self.reloading || self.sending.is_some() || self.held(revision) {
            return None;
        }
        if now - self.seen.1 < QUIET_MS {
            return None;
        }
        if self.failed_at.is_some_and(|at| now - at < RETRY_MS) {
            return None;
        }
        self.send(revision);
        Some(self.server)
    }

    /// The next step of saving everything now, asked for before a render:
    /// the quiet and the retry waits do not apply. When it is `Send`, the
    /// document at `revision` counts as on its way, as from `due`.
    pub(crate) fn flush(&mut self, revision: u64) -> Flush {
        if self.reloading {
            return Flush::Stuck;
        }
        if self.sending.is_some() {
            return Flush::Wait;
        }
        if self.held(revision) {
            return Flush::Kept(self.server);
        }
        self.send(revision);
        Flush::Send(self.server)
    }

    /// The save on its way was kept, as the service's `server` revision.
    pub(crate) fn saved(&mut self, server: i64) {
        if let Some(sent) = self.sending.take() {
            self.saved = sent;
        }
        self.sending_files = false;
        self.server = server;
    }

    /// The save on its way did not go through; the next try waits.
    pub(crate) fn failed(&mut self, now: f64) {
        self.sending = None;
        self.files_changed |= std::mem::take(&mut self.sending_files);
        self.failed_at = Some(now);
    }

    /// The save on its way was refused as stale: nothing more goes while the
    /// service's copy is fetched.
    pub(crate) fn conflicted(&mut self) {
        self.sending = None;
        self.sending_files = false;
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

/// What saving everything now (`Autosave::flush`) is to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flush {
    /// Nothing is left to send: the service holds the document as this
    /// revision.
    Kept(i64),
    /// A save is on its way: ask again once it is answered.
    Wait,
    /// Send the document now, on this service revision.
    Send(i64),
    /// The service's copy is being fetched over this one: nothing can be
    /// saved from here until it is open.
    Stuck,
}

/// What a tab answers the service's `flush`: the cloud project it is bound
/// to — by the service's id, from its `…/account/projects/{id}` address — and
/// the service revision holding the document; no project when it is bound to
/// none.
pub(crate) fn flush_answer(url: Option<&str>, revision: i64) -> serde_json::Value {
    match url {
        Some(url) => serde_json::json!({"project": project_id(url), "revision": revision}),
        None => serde_json::json!({"project": null}),
    }
}

/// The service's id of the cloud project at `url`.
fn project_id(url: &str) -> &str {
    let id = url
        .rsplit_once("/account/projects/")
        .map_or(url, |(_, id)| id);
    id.split(['?', '#']).next().unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::{Autosave, Flush, QUIET_MS, RETRY_MS, flush_answer};

    /// A file of the project replaced with no change to the document — a
    /// product's model the service sent — is saved like a change: after
    /// the quiet, or at once on a flush; and again after a failed try.
    #[test]
    fn a_replaced_file_is_saved_like_a_change() {
        let mut save = Autosave::new(7, 3, 0.0);
        save.files_changed();
        assert_eq!(save.due(3, 0.0), None, "the quiet first");
        assert_eq!(save.due(3, QUIET_MS), Some(7));
        save.failed(QUIET_MS);
        assert_eq!(save.due(3, QUIET_MS + RETRY_MS), Some(7), "tried again");
        save.saved(8);
        assert_eq!(save.due(3, 10.0 * RETRY_MS), None, "kept");
        assert_eq!(save.flush(3), Flush::Kept(8));
        save.files_changed();
        assert_eq!(save.flush(3), Flush::Send(8));
        save.saved(9);
        assert_eq!(save.flush(3), Flush::Kept(9));
    }

    #[test]
    fn a_flush_answer_names_the_project_by_its_id() {
        assert_eq!(
            flush_answer(Some("https://svc.example/account/projects/ab12"), 7),
            serde_json::json!({"project": "ab12", "revision": 7})
        );
        assert_eq!(
            flush_answer(Some("https://svc.example/account/projects/ab12?x=1#top"), 3),
            serde_json::json!({"project": "ab12", "revision": 3})
        );
        assert_eq!(flush_answer(None, 9), serde_json::json!({"project": null}));
    }

    #[test]
    fn a_flush_with_nothing_pending_answers_the_held_revision() {
        let mut save = Autosave::new(7, 3, 0.0);
        let before = save.clone();
        assert_eq!(save.flush(3), Flush::Kept(7));
        assert_eq!(save, before, "nothing changes");
    }

    #[test]
    fn a_flush_sends_a_fresh_change_without_waiting_for_quiet() {
        let mut save = Autosave::new(7, 3, 0.0);
        save.due(4, 100.0);
        assert_eq!(save.flush(4), Flush::Send(7), "at once, not after quiet");
        assert_eq!(save.due(4, 100.0 + 10.0 * QUIET_MS), None, "one save out");
        save.saved(8);
        assert_eq!(save.flush(4), Flush::Kept(8));
    }

    #[test]
    fn a_flush_waits_for_the_save_on_its_way_then_sends_what_changed_since() {
        let mut save = Autosave::new(7, 3, 0.0);
        save.due(4, 0.0);
        assert_eq!(save.due(4, QUIET_MS), Some(7));
        assert_eq!(save.flush(5), Flush::Wait);
        assert_eq!(save.flush(5), Flush::Wait, "still out");
        save.saved(8);
        assert_eq!(save.flush(5), Flush::Send(8), "the later change, at once");
        save.saved(9);
        assert_eq!(save.flush(5), Flush::Kept(9));
    }

    #[test]
    fn a_flush_does_not_wait_out_a_failed_save() {
        let mut save = Autosave::new(7, 3, 0.0);
        save.due(4, 0.0);
        assert_eq!(save.due(4, QUIET_MS), Some(7));
        save.failed(QUIET_MS);
        assert_eq!(save.flush(4), Flush::Send(7), "no retry wait");
        save.failed(QUIET_MS + 1.0);
        assert_eq!(
            save.due(4, QUIET_MS + 2.0),
            None,
            "the normal saves still wait"
        );
    }

    #[test]
    fn a_flush_while_the_service_copy_is_fetched_is_stuck() {
        let mut save = Autosave::new(7, 3, 0.0);
        save.due(4, 0.0);
        assert_eq!(save.due(4, QUIET_MS), Some(7));
        save.conflicted();
        assert_eq!(save.flush(4), Flush::Stuck);
        save.reload_failed(QUIET_MS);
        assert_eq!(
            save.flush(4),
            Flush::Send(7),
            "free again after the reload failed"
        );
    }

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
                files_changed: false,
                sending_files: false,
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
