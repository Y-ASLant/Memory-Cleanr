use memory_cleanr::optimize::MemoryAreas;
use memory_cleanr::settings::Settings;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

static APPDATA_TEST_LOCK: Mutex<()> = Mutex::new(());

fn with_temp_appdata<F>(run: F)
where
    F: FnOnce(&std::path::Path),
{
    let _guard = APPDATA_TEST_LOCK.lock().expect("app_data test lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let previous = std::env::var_os("APPDATA");
    // SAFETY: tests run serially by default and restore APPDATA before returning.
    unsafe {
        std::env::set_var("APPDATA", temp.path());
    }

    run(temp.path());

    if let Some(path) = previous {
        unsafe {
            std::env::set_var("APPDATA", path);
        }
    } else {
        unsafe {
            std::env::remove_var("APPDATA");
        }
    }
}

#[test]
fn settings_save_and_load_roundtrip_in_temp_config_dir() {
    with_temp_appdata(|app_data| {
        fs::create_dir_all(app_data.join("MemoryCleaner")).expect("create config dir");

        let settings = Settings {
            always_on_top: true,
            close_to_notification_area: false,
            show_virtual_memory: false,
            memory_areas: MemoryAreas::WORKING_SET.bits(),
            debug_logging: true,
            ..Settings::default()
        };
        settings.save();

        let loaded = Settings::load();
        assert_eq!(loaded.always_on_top, true);
        assert_eq!(loaded.close_to_notification_area, false);
        assert_eq!(loaded.show_virtual_memory, false);
        assert_eq!(loaded.memory_areas, MemoryAreas::WORKING_SET.bits());
        assert_eq!(loaded.debug_logging, true);
    });
}

#[test]
fn settings_save_uses_atomic_replace() {
    with_temp_appdata(|app_data| {
        Settings::default().save();

        let final_path: PathBuf = app_data.join("MemoryCleaner").join("settings.toml");
        let tmp_path = final_path.with_extension("toml.tmp");

        assert!(final_path.is_file());
        assert!(!tmp_path.exists());
    });
}
