//! Cross-reference table + the fixpoint verdict, a small port of
//! `crossRef.ml`. Owned by the compile driver (`lib.rs::compile_document_cst`),
//! **not** reset per trial — it *is* the fixpoint state that persists while
//! everything else (hooks, images, mutable store) resets each trial.
//!
//! docs/plans/hooks-annotations-crossref.md §Cross-references & the fixpoint.

use std::collections::HashMap;

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
    changed: bool,
    count: u32,
    /// Keys `get` missed this trial (`crossRef.ml:116`).
    unresolved: Vec<String>,
}

impl CrossRefs {
    pub fn new() -> CrossRefs {
        CrossRefs::default()
    }

    /// `crossRef.ml:99` — sets `changed` iff the key is new or its value
    /// differs from what's already stored.
    pub fn register(&mut self, k: String, v: String) {
        match self.table.get(&k) {
            Some(old) if *old == v => {}
            _ => {
                self.changed = true;
                self.table.insert(k, v);
            }
        }
    }

    /// `crossRef.ml:116` — records a miss (an unresolved forward reference
    /// forces a retrial) alongside the ordinary lookup.
    pub fn get(&mut self, k: &str) -> Option<String> {
        match self.table.get(k) {
            Some(v) => Some(v.clone()),
            None => {
                self.unresolved.push(k.to_string());
                None
            }
        }
    }

    /// `crossRef.ml:?` — like [`Self::get`] but records no miss (roadmap §A's
    /// `probe-cross-reference`).
    #[allow(dead_code)]
    pub fn probe(&self, k: &str) -> Option<String> {
        self.table.get(k).cloned()
    }

    /// `crossRef.ml:78` `needs_another_trial`. Consumes this trial's
    /// `changed`/`unresolved` bookkeeping and resets it for the next trial.
    pub fn verdict(&mut self) -> Verdict {
        if self.changed {
            if self.count >= COUNT_MAX {
                Verdict::CountMax
            } else {
                self.unresolved.clear();
                self.changed = false;
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
    fn a_stable_key_converges_on_the_second_trial() {
        let mut cr = CrossRefs::new();
        // Trial 1: register only — nothing read back yet, but a fresh key is
        // always "changed".
        cr.register("p".to_string(), "1".to_string());
        assert_eq!(cr.verdict(), Verdict::NeedsAnotherTrial);

        // Trial 2: register the same value again (a real trial re-runs the
        // whole document, so `register` fires again with an unchanged
        // value) and read it back successfully.
        cr.register("p".to_string(), "1".to_string());
        assert_eq!(cr.get("p"), Some("1".to_string()));
        assert_eq!(cr.verdict(), Verdict::CanTerminate(Vec::new()));
    }

    #[test]
    fn a_forward_reference_that_never_resolves_hits_count_max() {
        let mut cr = CrossRefs::new();
        for i in 0..(COUNT_MAX + 2) {
            // A pathological document that registers a *new* value every
            // trial never stabilizes; the cap must still terminate it.
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
}
