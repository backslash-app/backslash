//! Launch at login through the XDG autostart directory.

use std::path::PathBuf;

use anyhow::Result;

fn path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| backslash_core::paths::home().join(".config"))
        .join("autostart/app.backslash.Backslash.desktop")
}

pub fn enabled() -> bool {
    path().is_file()
}

pub fn set(enabled: bool) -> Result<()> {
    let path = path();
    if !enabled {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    std::fs::create_dir_all(path.parent().expect("autostart dir"))?;
    std::fs::write(
        &path,
        format!(
            "[Desktop Entry]\nType=Application\nName=Backslash\nComment=Start the launcher in the background\n\
             Exec={} --hidden\nIcon=app.backslash.Backslash\nTerminal=false\nNoDisplay=true\n\
             X-GNOME-Autostart-enabled=true\n",
            exe.display()
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn toggles_the_desktop_entry() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", dir.path());
        assert!(!super::enabled());
        super::set(true).unwrap();
        assert!(super::enabled());
        let entry = std::fs::read_to_string(dir.path().join("autostart/app.backslash.Backslash.desktop")).unwrap();
        assert!(entry.contains("--hidden"));
        super::set(false).unwrap();
        assert!(!super::enabled());
    }
}
