//! Desktop integrations that need no GUI: clipboard history storage, file
//! search, window listing.

pub mod clipboard;
pub mod files;
pub mod gnome;
pub mod windows;

/// First of `names` found in `$PATH`.
pub fn which(names: &[&str]) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    names.iter().find(|n| std::env::split_paths(&path).any(|d| d.join(n).is_file())).map(|n| n.to_string())
}
