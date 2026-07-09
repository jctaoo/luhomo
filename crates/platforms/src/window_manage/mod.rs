use anyhow::{Context, bail};
use raw_window_handle::HasWindowHandle;

#[cfg(target_os = "windows")]
mod win32;

pub fn disable_window_maximize(window: &impl HasWindowHandle) -> anyhow::Result<()> {
    let raw = window
        .window_handle()
        .map_err(anyhow::Error::msg)
        .context("getting raw window handle")?;

    let raw = raw.as_raw();

    #[cfg(target_os = "windows")]
    {
        win32::disable_window_maximize(raw).unwrap_or_else(|e| {
            eprintln!("win32 configure failed: {e}");
        });
    }

    bail!("Not support yet on current platform: disable_window_maximize");
}

pub fn configure_window_max_size(
    window: &impl HasWindowHandle,
    max_width: f32,
    max_height: f32,
) -> anyhow::Result<()> {
    // assert window size
    assert!(max_width > 0.0, "max_width must be greater than 0");
    assert!(max_height > 0.0, "max_height must be greater than 0");

    let raw = window
        .window_handle()
        .map_err(anyhow::Error::msg)
        .context("getting raw window handle")?;

    let raw = raw.as_raw();

    #[cfg(target_os = "windows")]
    {
        win32::configure_max_window_size(raw, max_width, max_height).unwrap_or_else(|e| {
            eprintln!("win32 configure failed: {e}");
        });
    }

    bail!("Not support yet on current platform: configure_window_max_size");
}
