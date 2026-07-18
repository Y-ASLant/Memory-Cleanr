use anyhow::Result;
use gpui::{KeyBinding, QuitMode, actions, *};

use crate::app::{self, AppEntityHolder};
use crate::settings::Settings;

actions!(wmc_gpui, [Quit]);

pub fn run(settings: Settings) -> Result<()> {
    gpui_platform::application()
        .with_assets(gpui_component_assets::Assets)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_component::init(cx);

            cx.bind_keys([KeyBinding::new("alt-f4", Quit, None)]);
            cx.on_action(|_quit: &Quit, cx: &mut App| {
                let entity = cx
                    .try_global::<AppEntityHolder>()
                    .map(|holder| holder.0.clone());
                if let Some(entity) = entity {
                    entity.update(cx, |app, cx| app.handle_window_close(cx));
                } else {
                    cx.defer(|cx| cx.quit());
                }
            });

            cx.spawn(async move |cx| {
                app::open_main_window(cx, settings).expect("Failed to open window");
            })
            .detach();
        });

    Ok(())
}
