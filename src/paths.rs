//! Known folders and screenshot file names.

use std::path::PathBuf;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Screenshots, SHGetKnownFolderPath, KF_FLAG_CREATE};

use crate::capture::Timestamp;

/// The Windows Screenshots folder ("Pictures\Screenshots"), created if
/// needed. It follows the user when the folder was moved, for example to
/// OneDrive.
pub fn screenshots_folder() -> Option<PathBuf> {
    // SAFETY: the returned string is copied, then freed with CoTaskMemFree
    // as the API requires.
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Screenshots, KF_FLAG_CREATE, None).ok()?;
        let result = path.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(path.0.cast_const().cast()));
        result
    }
}

/// The file name of a screenshot: `2026-11-25_14-32-10.png`, then
/// `2026-11-25_14-32-10_2.png` and so on when the name is taken.
pub fn file_name(time: Timestamp, attempt: u32) -> String {
    let base = format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        time.year, time.month, time.day, time.hour, time.minute, time.second
    );
    if attempt <= 1 {
        format!("{base}.png")
    } else {
        format!("{base}_{attempt}.png")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIME: Timestamp = Timestamp {
        year: 2026,
        month: 11,
        day: 5,
        hour: 9,
        minute: 3,
        second: 7,
    };

    #[test]
    fn names_sort_by_time() {
        assert_eq!(file_name(TIME, 1), "2026-11-05_09-03-07.png");
    }

    #[test]
    fn collisions_get_a_suffix() {
        assert_eq!(file_name(TIME, 2), "2026-11-05_09-03-07_2.png");
        assert_eq!(file_name(TIME, 10), "2026-11-05_09-03-07_10.png");
    }
}
