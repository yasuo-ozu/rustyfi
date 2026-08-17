//! Cross-reference table + the fixpoint verdict, a small port of
//! `crossRef.ml`. Owned by the compile driver (`lib.rs::compile_document_cst`),
//! **not** reset per trial — it *is* the fixpoint state that persists while
//! everything else (hooks, images, mutable store) resets each trial.
//!
//! docs/plans/hooks-annotations-crossref.md §Cross-references & the fixpoint.

use std::collections::{BTreeMap, HashMap, HashSet};

/// A cross-reference table as it is carried BETWEEN runs — the payload of the
/// auxiliary file (`<doc>.satysfi-aux`). `BTreeMap` so serializing it is
/// deterministic: the same table must always produce the same file bytes.
pub type AuxTable = BTreeMap<String, String>;

/// `crossRef.ml:23`'s `count_max`.
pub const COUNT_MAX: u32 = 4;

/// What the driver should do after one trial finished and hooks fired.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The table changed this trial; run another trial.
    NeedsAnotherTrial,
    /// The table stabilized (no `register` changed a value this trial).
    /// Carries the keys `get` missed during the *final* trial (unresolved
    /// forward references), mirroring `crossRef.ml:78`'s `needs_another_trial`
    /// returning the leftover unresolved list on termination.
    CanTerminate(Vec<String>),
    /// The table kept changing through `COUNT_MAX` trials; give up rather
    /// than loop forever.
    CountMax,
}

/// Port of `crossRef.ml`'s mutable cross-reference table.
#[derive(Debug, Default)]
pub struct CrossRefs {
    table: HashMap<String, String>,
    count: u32,
    /// Keys `get` missed this trial (`crossRef.ml:116`).
    unresolved: Vec<String>,
    /// What each key's value looked like when it was READ (`get`) this trial —
    /// `Some(v)` for a hit, `None` for a miss — recorded at the FIRST read.
    /// A trial's LAYOUT depends only on the cross-reference values it actually
    /// reads; this lets `verdict` retry only when a read value was invalidated,
    /// not merely because some key was (re)registered.
    read_this_trial: HashMap<String, Option<String>>,
    /// Set when a `register` this trial gives a key a value differing from what
    /// a `get` earlier this trial had already observed for it — i.e. the layout
    /// used a now-stale cross-reference and must be recomputed.
    stale: bool,
    /// Keys this table was SEEDED with from a previous run's auxiliary file
    /// (empty for a cold run). Tracked only to police them — see
    /// [`CrossRefs::seed_unvalidated`].
    seeded: HashSet<String>,
    /// Keys `register`ed during the trial now in progress. Reset by
    /// [`CrossRefs::verdict`] alongside the rest of the per-trial bookkeeping.
    registered_this_trial: HashSet<String>,
    /// Did the trial that just ended read a seeded value it never re-derived?
    /// See [`CrossRefs::seed_unvalidated`].
    seed_unvalidated: bool,
}

impl CrossRefs {
    pub fn new() -> CrossRefs {
        CrossRefs::default()
    }

    /// A table pre-populated from a previous run's auxiliary file.
    ///
    /// Seeding only changes how fast the fixpoint converges, never where it
    /// converges to: a seeded value a `get` observes and a later `register`
    /// contradicts still marks the layout stale and forces another trial,
    /// exactly as a value derived this run would. What seeding buys is the
    /// common case — a forward reference (`\ref` to a later section, a page
    /// number) reads the right answer on trial 1 instead of missing, being
    /// registered, and forcing trial 2.
    pub fn seeded(table: AuxTable) -> CrossRefs {
        CrossRefs {
            seeded: table.keys().cloned().collect(),
            table: table.into_iter().collect(),
            ..CrossRefs::default()
        }
    }

    /// The table as it should be written back out.
    ///
    /// Keys this run neither read nor registered are carried through
    /// verbatim, which is what lets an auxiliary file round-trip between this
    /// port and upstream SATySFi without either dropping the other's
    /// bookkeeping.
    pub fn export(&self) -> AuxTable {
        self.table
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Did the trial that just ended READ a seeded value that it never
    /// re-registered?
    ///
    /// If so the layout depended on a value carried in from a previous run
    /// and never re-derived — a document that dropped the `\label` some
    /// surviving `\ref` points at. The value is not wrong so much as
    /// unverifiable, and trusting it would make the output depend on whether
    /// an auxiliary file happened to exist. The driver's answer is to discard
    /// the seed and redo the fixpoint cold, so a warm build is always
    /// byte-identical to a cold one.
    pub fn seed_unvalidated(&self) -> bool {
        self.seed_unvalidated
    }

    /// `crossRef.ml:99`, plus the fixpoint-shortcut bookkeeping: if this key was
    /// already READ this trial and the value the reader saw differs from `v`,
    /// the layout is now stale and another trial is required. A key that is
    /// (re)registered but never read this trial does NOT force a retrial — its
    /// value cannot have affected the output — which is what lets a document
    /// that only *writes* cross-references (e.g. page labels nothing `\ref`s)
    /// converge in ONE trial instead of the old always-two.
    pub fn register(&mut self, k: String, v: String) {
        self.registered_this_trial.insert(k.clone());
        if let Some(observed) = self.read_this_trial.get(&k) {
            let unchanged = matches!(observed, Some(o) if *o == v);
            if !unchanged {
                self.stale = true;
            }
        }
        self.table.insert(k, v);
    }

    /// `crossRef.ml:116` — records a miss (an unresolved forward reference)
    /// alongside the ordinary lookup, and remembers what the reader observed so
    /// a later `register` can tell whether that observation went stale.
    pub fn get(&mut self, k: &str) -> Option<String> {
        match self.table.get(k) {
            Some(v) => {
                let v = v.clone();
                self.read_this_trial
                    .entry(k.to_string())
                    .or_insert_with(|| Some(v.clone()));
                Some(v)
            }
            None => {
                self.unresolved.push(k.to_string());
                self.read_this_trial.entry(k.to_string()).or_insert(None);
                None
            }
        }
    }

    /// `crossRef.ml:112` — like [`Self::get`] but records no miss
    /// (`probe-cross-reference`): probing an absent key must NOT force
    /// another fixpoint trial.
    pub fn probe(&self, k: &str) -> Option<String> {
        self.table.get(k).cloned()
    }

    /// `crossRef.ml:78` `needs_another_trial`, tightened: retry only if a value
    /// the layout READ this trial was invalidated (`stale`), not on every
    /// (re)registration. Consumes this trial's bookkeeping and resets it.
    pub fn verdict(&mut self) -> Verdict {
        // A seeded key the layout READ but this trial never re-registered is
        // an unverified dependency (see `seed_unvalidated`). Compute it before
        // the per-trial bookkeeping is cleared; only the FINAL trial's answer
        // matters, and each trial overwrites it.
        self.seed_unvalidated = self
            .read_this_trial
            .keys()
            .any(|k| self.seeded.contains(k) && !self.registered_this_trial.contains(k));
        self.read_this_trial.clear();
        self.registered_this_trial.clear();
        if self.stale {
            if self.count >= COUNT_MAX {
                Verdict::CountMax
            } else {
                self.unresolved.clear();
                self.stale = false;
                self.count += 1;
                Verdict::NeedsAnotherTrial
            }
        } else {
            Verdict::CanTerminate(std::mem::take(&mut self.unresolved))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_only_key_converges_on_the_first_trial() {
        let mut cr = CrossRefs::new();
        // A key that is registered but never READ this trial cannot have
        // affected the layout, so the fixpoint terminates immediately — no
        // wasted confirmation trial (the whole point of the shortcut).
        cr.register("p".to_string(), "1".to_string());
        assert_eq!(cr.verdict(), Verdict::CanTerminate(Vec::new()));
    }

    #[test]
    fn a_forward_reference_needs_a_second_trial_then_converges() {
        let mut cr = CrossRefs::new();
        // Trial 1: read "p" before it exists (a forward ref) — the layout saw
        // `None` — then register it. The observed value went stale, so retry.
        assert_eq!(cr.get("p"), None);
        cr.register("p".to_string(), "1".to_string());
        assert_eq!(cr.verdict(), Verdict::NeedsAnotherTrial);

        // Trial 2: the read now hits "1", and the re-registration matches it —
        // nothing the layout read changed, so terminate.
        assert_eq!(cr.get("p"), Some("1".to_string()));
        cr.register("p".to_string(), "1".to_string());
        assert_eq!(cr.verdict(), Verdict::CanTerminate(Vec::new()));
    }

    #[test]
    fn a_read_reference_that_never_resolves_hits_count_max() {
        let mut cr = CrossRefs::new();
        for i in 0..(COUNT_MAX + 2) {
            // A pathological document that READS a key and then registers a
            // *new* value for it every trial never stabilizes; the cap must
            // still terminate it. (The read is what makes the change matter —
            // a write-only churn would harmlessly converge on trial 1.)
            let _ = cr.get("k");
            cr.register("k".to_string(), i.to_string());
            let v = cr.verdict();
            if v == Verdict::CountMax {
                return;
            }
        }
        panic!("expected CountMax within COUNT_MAX trials");
    }

    #[test]
    fn an_unresolved_get_forces_another_trial_via_the_caller_not_verdict_alone() {
        // `get` missing a key does not, by itself, set `changed` — only
        // `register` does (matching `crossRef.ml`: the "needs another
        // trial" signal is entirely the `changed` flag; the unresolved list
        // is informational/diagnostic). A document that never registers the
        // key it `get`s therefore converges immediately, with the miss
        // surfaced in `CanTerminate`'s payload.
        let mut cr = CrossRefs::new();
        assert_eq!(cr.get("missing"), None);
        assert_eq!(
            cr.verdict(),
            Verdict::CanTerminate(vec!["missing".to_string()])
        );
    }

    #[test]
    fn seeding_resolves_a_forward_reference_on_the_first_trial() {
        // Cold, this is the two-trial case (see the test above): the layout
        // reads "p" before anything registers it, so trial 1's observation
        // goes stale. Seeded from the previous run's table, the same read hits
        // the right value immediately and the re-registration confirms it —
        // one trial, same answer.
        let mut aux = AuxTable::new();
        aux.insert("p".to_string(), "1".to_string());
        let mut cr = CrossRefs::seeded(aux);

        assert_eq!(cr.get("p"), Some("1".to_string()));
        cr.register("p".to_string(), "1".to_string());
        assert_eq!(cr.verdict(), Verdict::CanTerminate(Vec::new()));
        assert!(!cr.seed_unvalidated(), "the seed was re-derived this run");
    }

    #[test]
    fn a_seed_the_run_contradicts_still_forces_another_trial() {
        // Seeding must not be able to freeze a wrong answer: a seeded value the
        // layout reads and a later `register` contradicts is exactly the stale
        // case, and retries just as an in-run value would.
        let mut aux = AuxTable::new();
        aux.insert("p".to_string(), "STALE".to_string());
        let mut cr = CrossRefs::seeded(aux);

        assert_eq!(cr.get("p"), Some("STALE".to_string()));
        cr.register("p".to_string(), "1".to_string());
        assert_eq!(cr.verdict(), Verdict::NeedsAnotherTrial);
    }

    #[test]
    fn a_seed_that_is_read_but_never_re_registered_is_flagged() {
        // The dangerous shape: the document still `\ref`s a key but no longer
        // defines it, so nothing this run can confirm the seeded value. The
        // fixpoint converges happily — which is the problem — so `verdict`
        // reports it and the driver redoes the run cold rather than emit a
        // layout that depends on a file being present.
        let mut aux = AuxTable::new();
        aux.insert("p".to_string(), "1".to_string());
        let mut cr = CrossRefs::seeded(aux);

        assert_eq!(cr.get("p"), Some("1".to_string()));
        assert_eq!(cr.verdict(), Verdict::CanTerminate(Vec::new()));
        assert!(
            cr.seed_unvalidated(),
            "read a seeded value nothing re-derived"
        );
    }

    #[test]
    fn an_unread_seed_is_not_flagged_and_is_carried_through() {
        // A seeded key the document never reads cannot have affected the
        // layout, so it is no reason to redo anything — but it is still
        // written back out, which is what lets an auxiliary file round-trip
        // between this port and upstream SATySFi (whose own `changed` marker
        // lives in the same table) without either dropping the other's keys.
        let mut aux = AuxTable::new();
        aux.insert("changed".to_string(), "F".to_string());
        aux.insert("stale-label".to_string(), "9".to_string());
        let mut cr = CrossRefs::seeded(aux);

        cr.register("other".to_string(), "1".to_string());
        assert_eq!(cr.verdict(), Verdict::CanTerminate(Vec::new()));
        assert!(!cr.seed_unvalidated());

        let out = cr.export();
        assert_eq!(out.get("changed").map(String::as_str), Some("F"));
        assert_eq!(out.get("stale-label").map(String::as_str), Some("9"));
        assert_eq!(out.get("other").map(String::as_str), Some("1"));
    }
}
