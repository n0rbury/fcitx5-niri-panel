use fcitx5_niri_panel::model::{CandidateLayout, PanelState, Rect};

#[test]
fn lookup_table_is_normalized() {
    let mut state = PanelState::default();
    state
        .set_lookup_table(
            vec!["1".into(), "2".into()],
            vec!["你".into(), "呢".into()],
            vec!["0".into(), "0".into()],
            true,
            false,
            1,
            1,
        )
        .unwrap();

    assert_eq!(state.candidates.len(), 2);
    assert_eq!(state.candidates[1].text, "呢");
    assert_eq!(state.selected, 1);
    assert!(state.has_previous);
    assert!(!state.has_next);
    assert_eq!(state.layout, CandidateLayout::Vertical);
    assert!(state.visible);
    assert!(state.aux_down.is_empty());
}

#[test]
fn mismatched_lookup_arrays_are_rejected() {
    let mut state = PanelState::default();
    assert!(state
        .set_lookup_table(
            vec!["1".into()],
            vec!["你".into(), "呢".into()],
            vec!["0".into()],
            false,
            false,
            -1,
            0,
        )
        .is_err());
}

#[test]
fn aux_down_row_is_split_off() {
    let mut state = PanelState::default();
    state
        .set_lookup_table(
            vec!["".into(), "1.".into(), "2.".into()],
            vec!["拼音: nihao".into(), "你".into(), "呢".into()],
            vec!["".into(), "".into(), "".into()],
            true,
            false,
            2,
            1,
        )
        .unwrap();

    assert_eq!(state.aux_down, "拼音: nihao");
    assert_eq!(state.candidates.len(), 2);
    assert_eq!(state.candidates[0].text, "你");
    // Fcitx's pos included the aux row (row 0), so the normalized selected
    // index is shifted down by one.
    assert_eq!(state.selected, 1);
    assert!(state.visible);
}

#[test]
fn empty_lookup_table_clears_state() {
    let mut state = PanelState::default();
    state
        .set_lookup_table(
            vec!["1.".into()],
            vec!["你".into()],
            vec!["".into()],
            false,
            false,
            0,
            1,
        )
        .unwrap();
    assert!(state.visible);

    state
        .set_lookup_table(
            vec![],
            vec![],
            vec![],
            false,
            false,
            -1,
            0,
        )
        .unwrap();
    assert!(state.candidates.is_empty());
    assert!(state.aux_down.is_empty());
    assert_eq!(state.selected, -1);
    assert!(!state.visible);
}

#[test]
fn spot_and_scale_are_preserved() {
    let mut state = PanelState::default();
    state.spot = Some(Rect {
        x: 10,
        y: 20,
        width: 1,
        height: 22,
    });
    state.scale = Some(1.5);
    assert_eq!(state.spot.unwrap().x, 10);
    assert_eq!(state.scale, Some(1.5));
}
