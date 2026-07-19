use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::Foundation::MAX_PATH;
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::InitPropVariantFromStringAsVector;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
};
use windows::Win32::UI::Shell::{
    IShellLinkW, PropertiesSystem::IPropertyStore, SetCurrentProcessExplicitAppUserModelID,
    ShellLink,
};
use windows::core::{HSTRING, Interface};

pub const APP_USER_MODEL_ID: &str = "MemoryCleaner.App";

pub fn init() -> Result<()> {
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(APP_USER_MODEL_ID))
            .context("SetCurrentProcessExplicitAppUserModelID failed")?;
    }
    ensure_start_menu_shortcut()?;
    Ok(())
}

pub fn show(title: &str, body: &str) -> Result<()> {
    let xml = format!(
        r#"<toast><visual><binding template="ToastText02"><text id="1">{}</text><text id="2">{}</text></binding></visual></toast>"#,
        escape_xml(title),
        escape_xml(body),
    );

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let doc = XmlDocument::new().context("XmlDocument::new failed")?;
        doc.LoadXml(&HSTRING::from(xml))
            .context("toast XML load failed")?;

        let toast = ToastNotification::CreateToastNotification(&doc)
            .context("CreateToastNotification failed")?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_USER_MODEL_ID))?
            .Show(&toast)
            .context("ToastNotifier::Show failed")?;
    }

    Ok(())
}

fn ensure_start_menu_shortcut() -> Result<()> {
    let shortcut_path = start_menu_shortcut_path()?;
    if shortcut_path.is_file() {
        return Ok(());
    }

    let exe = std::env::current_exe().context("current_exe failed")?;
    if let Some(parent) = shortcut_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();

        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).context("ShellLink failed")?;
        link.SetPath(&HSTRING::from(exe.as_os_str()))
            .context("SetPath failed")?;
        link.SetArguments(&HSTRING::from(""))
            .context("SetArguments failed")?;

        let property_store: IPropertyStore = link.cast().context("IPropertyStore cast failed")?;
        let app_id = InitPropVariantFromStringAsVector(&HSTRING::from(APP_USER_MODEL_ID))
            .context("InitPropVariantFromStringAsVector failed")?;
        property_store
            .SetValue(&PKEY_AppUserModel_ID, &app_id)
            .context("SetValue PKEY_AppUserModel_ID failed")?;
        property_store
            .Commit()
            .context("property store Commit failed")?;

        let persist_file: IPersistFile = link.cast().context("IPersistFile cast failed")?;
        persist_file
            .Save(&HSTRING::from(shortcut_path.as_os_str()), true)
            .context("shortcut Save failed")?;
    }

    Ok(())
}

fn start_menu_shortcut_path() -> Result<PathBuf> {
    let mut appdata = [0u16; MAX_PATH as usize];
    let len = unsafe {
        windows::Win32::System::Environment::GetEnvironmentVariableW(
            windows::core::w!("APPDATA"),
            Some(&mut appdata),
        )
    };
    if len == 0 {
        anyhow::bail!("APPDATA not set");
    }

    let base = OsString::from_wide(&appdata[..len as usize]);
    Ok(PathBuf::from(base)
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join("Memory Cleaner.lnk"))
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_xml_escapes_special_chars() {
        assert_eq!(escape_xml("a & b"), "a &amp; b");
        assert_eq!(escape_xml("<tag>"), "&lt;tag&gt;");
    }
}
