use memory_cleanr::app::{CloseAction, resolve_close_action};

#[test]
fn resolve_close_action_honors_close_to_tray_setting() {
    assert_eq!(resolve_close_action(true), CloseAction::ReturnToTray);
    assert_eq!(resolve_close_action(false), CloseAction::ExitApp);
}
