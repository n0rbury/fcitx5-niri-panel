use crate::model::{Candidate, PanelState, Rect};
use anyhow::Result;
use std::sync::{Arc, RwLock};

/// Narrow, protocol-agnostic state sink used by the eventual D-Bus adapter.
///
/// Keeping this separate from the renderer is intentional: the protocol adapter
/// should only translate Fcitx/Kimpanel messages into PanelState mutations.
#[derive(Clone, Default)]
pub struct StateStore {
    inner: Arc<RwLock<PanelState>>,
}

impl StateStore {
    pub fn snapshot(&self) -> PanelState {
        self.inner.read().expect("state lock poisoned").clone()
    }

    pub fn set_visible(&self, visible: bool) {
        self.inner.write().expect("state lock poisoned").visible = visible;
    }

    pub fn set_preedit(&self, text: impl Into<String>) {
        self.inner.write().expect("state lock poisoned").preedit = text.into();
    }

    pub fn set_aux(&self, text: impl Into<String>) {
        self.inner.write().expect("state lock poisoned").aux = text.into();
    }

    pub fn set_spot(&self, rect: Rect) {
        self.inner.write().expect("state lock poisoned").spot = Some(rect);
    }

    pub fn clear_spot(&self) {
        self.inner.write().expect("state lock poisoned").spot = None;
    }

    pub fn set_candidates(&self, candidates: Vec<Candidate>, selected: usize) {
        let mut state = self.inner.write().expect("state lock poisoned");
        state.candidates = candidates;
        state.selected = selected.min(state.candidates.len().saturating_sub(1));
    }

    pub fn set_selected(&self, selected: usize) {
        let mut state = self.inner.write().expect("state lock poisoned");
        state.selected = selected.min(state.candidates.len().saturating_sub(1));
    }

    pub fn clear_lookup(&self) {
        self.inner.write().expect("state lock poisoned").clear_lookup();
    }
}

/// Translate a Kimpanel-style lookup table payload into our normalized model.
///
/// This is intentionally not tied to a concrete D-Bus crate yet. That lets us
/// lock down the state semantics and tests before depending on generated D-Bus
/// bindings and the exact Fcitx introspection ABI.
pub fn update_lookup_table(
    store: &StateStore,
    labels: &[impl AsRef<str>],
    texts: &[impl AsRef<str>],
    hints: &[impl AsRef<str>],
    selected: usize,
) -> Result<()> {
    if labels.len() != texts.len() || labels.len() != hints.len() {
        anyhow::bail!("lookup table vectors must have equal length");
    }

    let candidates = labels
        .iter()
        .zip(texts)
        .zip(hints)
        .map(|((label, text), hint)| Candidate {
            label: label.as_ref().to_owned(),
            text: text.as_ref().to_owned(),
            hint: hint.as_ref().to_owned(),
        })
        .collect();

    store.set_candidates(candidates, selected);
    Ok(())
}
