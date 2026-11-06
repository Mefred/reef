//! Slint application wiring (Phase 4).
//!
//! Wraps the generated Slint `MainWindow` component.

slint::include_modules!();

pub struct AppWindow {
    window: MainWindow,
}

impl AppWindow {
    pub fn new() -> Result<Self, slint::PlatformError> {
        let window = MainWindow::new()?;
        Ok(AppWindow { window })
    }

    pub fn show(self) -> Result<(), slint::PlatformError> {
        self.window.show()
    }
}
