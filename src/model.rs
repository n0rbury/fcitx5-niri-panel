use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub label: String,
    pub text: String,
    pub attr: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateLayout {
    NotSet = 0,
    Vertical = 1,
    Horizontal = 2,
    Table = 3,
}

impl Default for CandidateLayout {
    fn default() -> Self {
        Self::NotSet
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PanelState {
    pub visible: bool,
    pub preedit: String,
    pub preedit_visible: bool,
    pub preedit_cursor: i32,
    pub aux: String,
    pub aux_visible: bool,
    /// Auxiliary down line, transmitted by Fcitx as row 0 of SetLookupTable
    /// with an empty label. Kept separate from candidates.
    pub aux_down: String,
    pub candidates: Vec<Candidate>,
    pub selected: i32,
    pub has_previous: bool,
    pub has_next: bool,
    pub layout: CandidateLayout,
    pub spot: Option<Rect>,
    pub scale: Option<f64>,
}

impl PanelState {
    pub fn recompute_visible(&mut self) {
        self.visible = self.preedit_visible || self.aux_visible || !self.candidates.is_empty();
    }

    pub fn set_lookup_table(
        &mut self,
        labels: Vec<String>,
        texts: Vec<String>,
        attrs: Vec<String>,
        has_previous: bool,
        has_next: bool,
        selected: i32,
        layout: i32,
    ) -> anyhow::Result<()> {
        let n = labels.len();
        if texts.len() != n || attrs.len() != n {
            anyhow::bail!(
                "SetLookupTable arrays have different lengths: labels={}, texts={}, attrs={}",
                n,
                texts.len(),
                attrs.len(),
            );
        }

        // Fcitx transmits the auxiliary-down line as row 0 with an empty
        // label. Split it off before normalizing the candidate rows.
        let mut aux_down = String::new();
        let mut selected = selected;
        if let (Some(label0), Some(text0)) = (labels.first(), texts.first()) {
            if label0.is_empty() {
                aux_down = text0.clone();
                let mut labels = labels.into_iter();
                let mut texts = texts.into_iter();
                let mut attrs = attrs.into_iter();
                labels.next();
                texts.next();
                attrs.next();
                self.candidates = labels
                    .zip(texts)
                    .zip(attrs)
                    .map(|((label, text), attr)| Candidate { label, text, attr })
                    .collect();
                if selected > 0 {
                    selected -= 1;
                }
            } else {
                self.candidates = labels
                    .into_iter()
                    .zip(texts)
                    .zip(attrs)
                    .map(|((label, text), attr)| Candidate { label, text, attr })
                    .collect();
            }
        } else {
            self.candidates = Vec::new();
            selected = -1;
        }
        self.aux_down = aux_down;
        self.selected = selected;
        self.has_previous = has_previous;
        self.has_next = has_next;
        self.layout = match layout {
            1 => CandidateLayout::Vertical,
            2 => CandidateLayout::Horizontal,
            3 => CandidateLayout::Table,
            _ => CandidateLayout::NotSet,
        };
        self.recompute_visible();
        Ok(())
    }
}
