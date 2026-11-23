//! Native file dialogs on Windows (rfd). On other platforms the UI falls back to path text fields.

pub fn can_browse() -> bool {
    cfg!(windows)
}

#[cfg(windows)]
pub fn pick_file(filters: &[(&str, &[&str])]) -> Option<String> {
    let mut d = rfd::FileDialog::new();
    for (n, ext) in filters {
        d = d.add_filter(*n, ext);
    }
    d.pick_file().map(|p| p.to_string_lossy().into_owned())
}

#[cfg(windows)]
pub fn pick_files(filters: &[(&str, &[&str])]) -> Vec<String> {
    let mut d = rfd::FileDialog::new();
    for (n, ext) in filters {
        d = d.add_filter(*n, ext);
    }
    d.pick_files().unwrap_or_default().into_iter().map(|p| p.to_string_lossy().into_owned()).collect()
}

#[cfg(windows)]
pub fn save_file(default_name: &str, filters: &[(&str, &[&str])]) -> Option<String> {
    let mut d = rfd::FileDialog::new().set_file_name(default_name);
    for (n, ext) in filters {
        d = d.add_filter(*n, ext);
    }
    d.save_file().map(|p| p.to_string_lossy().into_owned())
}

#[cfg(windows)]
pub fn pick_folder() -> Option<String> {
    rfd::FileDialog::new().pick_folder().map(|p| p.to_string_lossy().into_owned())
}

#[cfg(not(windows))]
pub fn pick_file(_: &[(&str, &[&str])]) -> Option<String> {
    None
}
#[cfg(not(windows))]
pub fn pick_files(_: &[(&str, &[&str])]) -> Vec<String> {
    vec![]
}
#[cfg(not(windows))]
pub fn save_file(_: &str, _: &[(&str, &[&str])]) -> Option<String> {
    None
}
#[cfg(not(windows))]
pub fn pick_folder() -> Option<String> {
    None
}

pub fn open_folder(path: &std::path::Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}
