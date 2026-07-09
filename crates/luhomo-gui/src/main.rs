use gpui::*;

struct RootView;

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .items_center()
            .bg(rgb(0x1e1e2e))
            .text_xl()
            .text_color(rgb(0xcdd6f4))
            .child("Hello from Luhomo!")
    }
}

fn main() {
    Application::new().run(|app: &mut App| {
        app.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::centered(
                    size(px(800.0), px(600.0)),
                    app,
                )),
                titlebar: Some(TitlebarOptions {
                    title: Some("Luhomo".into()),
                    appears_transparent: false,
                    ..Default::default()
                }),
                app_id: Some("com.luhomo.app".into()),
                window_min_size: Some(size(px(400.0), px(300.0))),
                ..Default::default()
            },
            |_window, app| {
                app.new(|_cx| RootView)
            },
        )
        .unwrap();
    });
}
