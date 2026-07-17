use memory_cleanr::win32::process::{gui_instance_launch_spec, tray_instance_launch_spec};
use memory_cleanr::win32::startup::STARTUP_ARG;

#[test]
fn tray_launch_spec_includes_startup_flag() {
    let spec = tray_instance_launch_spec().expect("current_exe");
    assert_eq!(spec.args, vec![STARTUP_ARG.to_string()]);
}

#[test]
fn gui_launch_spec_has_no_startup_flag() {
    let spec = gui_instance_launch_spec().expect("current_exe");
    assert!(spec.args.is_empty());
}

#[test]
fn tray_and_gui_launch_specs_use_same_executable() {
    let tray = tray_instance_launch_spec().expect("tray");
    let gui = gui_instance_launch_spec().expect("gui");
    assert_eq!(tray.exe, gui.exe);
}
