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
    pub hint: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelState {
    pub visible: bool,
    pub preedit: String,
    pub aux: String,
    pub candidates: Vec<Candidate>,
    pub selected: usize,
    pub spot: Option<Rect>,
}

impl PanelState {
    pub fn clear_lookup(&mut self) {
        self.candidates.clear();
        self.selected = 0;
    }
}
