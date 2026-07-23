use anyhow::{Context, Result};
use gpui::{Pixels, Point, point, px};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// Current cursor position in screen coordinates.
pub fn screen_point() -> Result<Point<Pixels>> {
    unsafe {
        let mut pt = POINT::default();
        GetCursorPos(&mut pt).context("GetCursorPos")?;
        Ok(point(px(pt.x as f32), px(pt.y as f32)))
    }
}
