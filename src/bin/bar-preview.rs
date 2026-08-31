//! Offline preview: render sample panel states to PPM files with no Wayland
//! connection, for visual inspection (convert to PNG with ImageMagick).

use std::fs::File;
use std::io::Write;

use cosmic_text::{FontSystem, SwashCache};
use fcitx5_niri_panel::model::{Candidate, CandidateLayout, PanelState};
use fcitx5_niri_panel::render::{estimate_bar_width, render_bar_pixels};

fn state_vertical() -> PanelState {
    let mut s = PanelState::default();
    s.preedit = "zhong guo".into();
    s.preedit_visible = true;
    s.preedit_cursor = 9;
    s.candidates = vec![
        Candidate { label: "1 ".into(), text: "中古".into(), attr: String::new() },
        Candidate { label: "2 ".into(), text: "钟鼓".into(), attr: String::new() },
        Candidate { label: "3 ".into(), text: "中谷".into(), attr: String::new() },
        Candidate { label: "4 ".into(), text: "忠骨".into(), attr: String::new() },
        Candidate { label: "5 ".into(), text: "终古".into(), attr: String::new() },
        Candidate { label: "6 ".into(), text: "中".into(), attr: String::new() },
    ];
    s.selected = 0;
    s.has_next = true;
    s.layout = CandidateLayout::Vertical;
    s.recompute_visible();
    s
}

fn state_aux_down() -> PanelState {
    let mut s = PanelState::default();
    s.aux_down = "拼音: ni hao".into();
    s.candidates = vec![
        Candidate { label: "1 ".into(), text: "你好".into(), attr: String::new() },
        Candidate { label: "2 ".into(), text: "你会".into(), attr: String::new() },
        Candidate { label: "3 ".into(), text: "你还".into(), attr: String::new() },
        Candidate { label: "4 ".into(), text: "腻害".into(), attr: String::new() },
        Candidate { label: "5 ".into(), text: "你和".into(), attr: String::new() },
        Candidate { label: "6 ".into(), text: "你花".into(), attr: String::new() },
    ];
    s.selected = 2;
    s.has_next = true;
    s.has_previous = true;
    s.layout = CandidateLayout::Vertical;
    s.recompute_visible();
    s
}

fn state_horizontal() -> PanelState {
    let mut s = PanelState::default();
    s.aux = "辅助: 拼音".into();
    s.aux_visible = true;
    s.candidates = vec![
        Candidate { label: "1 ".into(), text: "中国".into(), attr: String::new() },
        Candidate { label: "2 ".into(), text: "种过".into(), attr: String::new() },
        Candidate { label: "3 ".into(), text: "重过".into(), attr: String::new() },
        Candidate { label: "4 ".into(), text: "种果".into(), attr: String::new() },
        Candidate { label: "5 ".into(), text: "中古".into(), attr: String::new() },
    ];
    s.selected = 1;
    s.has_next = true;
    s.layout = CandidateLayout::Horizontal;
    s.recompute_visible();
    s
}

fn write_ppm(path: &str, w: u32, h: u32, bgra: &[u8]) {
    let mut file = File::create(path).expect("create ppm");
    writeln!(file, "P6").unwrap();
    writeln!(file, "{w} {h}").unwrap();
    writeln!(file, "255").unwrap();
    let mut rgb = Vec::with_capacity(bgra.len() / 4 * 3);
    for px in bgra.chunks_exact(4) {
        let a = px[3] as u16;
        // Straight alpha over black.
        rgb.push(((px[2] as u16 * a / 255) & 0xff) as u8);
        rgb.push(((px[1] as u16 * a / 255) & 0xff) as u8);
        rgb.push(((px[0] as u16 * a / 255) & 0xff) as u8);
    }
    file.write_all(&rgb).unwrap();
}

fn main() {
    let mut fs = FontSystem::new();
    let mut swash = SwashCache::new();
    for (name, state) in [
        ("vertical", state_vertical()),
        ("aux_down", state_aux_down()),
        ("horizontal", state_horizontal()),
    ] {
        let w = estimate_bar_width(&state, &mut fs).max(1);
        // Rows actually built: vertical = preedit + 6 candidates;
        // aux_down = aux row + 6 candidates; horizontal = aux + 1 row.
        let row_count = match name {
            "vertical" | "aux_down" => 7,
            _ => 2,
        };
        let h = row_count * 22;
        let px = render_bar_pixels(&state, w, h, &mut fs, &mut swash).expect("render");
        let path = format!("/tmp/bar-{name}.ppm");
        write_ppm(&path, w, h, &px);
        println!("{path}");
    }
}
