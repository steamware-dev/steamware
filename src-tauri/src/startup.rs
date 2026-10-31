//! "Start with Windows", the one setting that lives outside the profile.
//!
//! A single value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, which is what the old
//! standalone wrote and what Task Manager's Startup tab reads. HKCU rather than HKLM on purpose: starting
//! with the user's session needs no administrator, and an app that cannot elevate should not ask to.
//!
//! The value is the executable's quoted absolute path, so a Steamware that was moved or reinstalled simply
//! updates the entry the next time this runs instead of leaving a dead command behind.

/// The Run key, and the value name inside it. The name is the app's, so removing the setting removes the
/// entry a user sees in Task Manager.
#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const VALUE_NAME: &str = "Steamware";

/// Write (or remove) the Run entry. Best effort by design: a locked-down machine that refuses the registry
/// write must still save the user's other settings, so the caller reports this rather than failing on it.
#[cfg(windows)]
pub fn apply(enabled: bool) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};
    use winreg::RegKey;

    let exe = std::env::current_exe()
        .map_err(|error| format!("Could not read Steamware's own path: {error}"))?;
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let key = root
        .open_subkey_with_flags(RUN_KEY, KEY_QUERY_VALUE | KEY_SET_VALUE)
        .map_err(|error| format!("Could not open the startup registry key: {error}"))?;

    if enabled {
        key.set_value(VALUE_NAME, &command_line(&exe.to_string_lossy()))
            .map_err(|error| format!("Could not add the startup entry: {error}"))
    } else {
        match key.delete_value(VALUE_NAME) {
            Ok(()) => Ok(()),
            // Nothing to remove is the state the user asked for, not a failure.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("Could not remove the startup entry: {error}")),
        }
    }
}

#[cfg(not(windows))]
pub fn apply(_enabled: bool) -> Result<(), String> {
    Ok(())
}

/// The Run value: a quoted absolute path, so a Steam folder with a space in it still starts. Public for the
/// test that pins the quoting.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn command_line(exe: &str) -> String {
    format!("\"{}\"", exe.trim_matches('"'))
}

/// Whether the entry currently exists and points at `exe`. Read-only, used to keep the switch honest when a
/// user removed the entry by hand (Task Manager can).
#[cfg(windows)]
pub fn registered() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE};
    use winreg::RegKey;

    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = root.open_subkey_with_flags(RUN_KEY, KEY_QUERY_VALUE) else {
        return false;
    };
    let Ok(value) = key.get_value::<String, _>(VALUE_NAME) else {
        return false;
    };
    value.trim_matches('"').eq_ignore_ascii_case(exe.to_string_lossy().as_ref())
}

#[cfg(not(windows))]
pub fn registered() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_value_quotes_the_path_and_does_not_double_it() {
        assert_eq!(command_line(r"C:\Steamware\steamware.exe"), r#""C:\Steamware\steamware.exe""#);
        assert_eq!(
            command_line(r#""C:\Program Files\Steamware\steamware.exe""#),
            r#""C:\Program Files\Steamware\steamware.exe""#
        );
    }
}
