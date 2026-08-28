use fcitx5_niri_panel::kimpanel::{update_lookup_table, StateStore};
use fcitx5_niri_panel::model::Rect;

#[test]
fn lookup_state_is_normalized() {
    let store = StateStore::default();
    update_lookup_table(
        &store,
        &["1", "2"],
        &["你", "呢"],
        &["a", "b"],
        1,
    )
    .unwrap();

    let state = store.snapshot();
    assert_eq!(state.candidates.len(), 2);
    assert_eq!(state.candidates[1].text, "呢");
    assert_eq!(state.candidates[0].hint, "a");
    assert_eq!(state.selected, 1);
}

#[test]
fn selected_index_is_clamped() {
    let store = StateStore::default();
    update_lookup_table(&store, &["1"], &["你"], &[""] , 99).unwrap();
    assert_eq!(store.snapshot().selected, 0);
}

#[test]
fn mismatched_lookup_vectors_are_rejected() {
    let store = StateStore::default();
    assert!(update_lookup_table(&store, &["1"], &["你", "呢"], &[""], 0).is_err());
}

#[test]
fn visibility_and_spot_are_independent() {
    let store = StateStore::default();
    store.set_visible(true);
    store.set_spot(Rect { x: 10, y: 20, width: 1, height: 22 });

    let state = store.snapshot();
    assert!(state.visible);
    assert_eq!(state.spot.unwrap().x, 10);
}
