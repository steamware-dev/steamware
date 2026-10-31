//! Windows DPAPI sealing for the one credential this app holds.
//!
//! The Hubcap key belongs to the person using it: they type it into Settings and it is spent on their own
//! Hubcap account. It has to survive a restart, so it cannot live only in memory — but writing it in the
//! clear into `%APPDATA%` would leave it for anything running as that user to read: a backup tool, a sync
//! client, a screenshot of an opened folder.
//!
//! `CryptProtectData` is the platform's own answer. Windows derives the encryption key from the logged-on
//! user's credentials, so the sealed blob is readable only by that user on that machine. Nothing secret is
//! shipped in the binary: a bundled key would be shared by every install and would be worth nothing.
//!
//! The entropy argument is an app-specific salt. DPAPI already keeps other *users* out; the salt also stops
//! another program running as the same user from decrypting this blob with a bare `CryptUnprotectData` call.
//!
//! Failing to seal is never a reason to fall back to plaintext: `seal` returns `None`, the key then stays in
//! memory for this run and is simply not written to disk.

use base64::Engine as _;

/// App-specific salt. Changing this string makes every blob written by an older build unreadable.
const ENTROPY: &[u8] = b"steamware.hubcap.key.v1";

/// Marks a sealed value so a plaintext field written by an older build is never mistaken for one.
const TAG: &str = "dpapi1:";

#[cfg(windows)]
mod dpapi {
    use super::ENTROPY;

    /// `DATA_BLOB` from `dpapi.h`: a length and a pointer to the bytes.
    #[repr(C)]
    struct Blob {
        size: u32,
        data: *mut u8,
    }

    /// Never show a UI prompt. This call happens at startup and on save; a modal from a background thread
    /// would hang the app.
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    // Declared by hand rather than pulled in through a crate: this is the whole of what the app needs from
    // DPAPI, and the two signatures have been stable since Windows 2000.
    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *const Blob,
            description: *const u16,
            entropy: *const Blob,
            reserved: *const core::ffi::c_void,
            prompt: *const core::ffi::c_void,
            flags: u32,
            data_out: *mut Blob,
        ) -> i32;

        fn CryptUnprotectData(
            data_in: *const Blob,
            description: *mut *mut u16,
            entropy: *const Blob,
            reserved: *const core::ffi::c_void,
            prompt: *const core::ffi::c_void,
            flags: u32,
            data_out: *mut Blob,
        ) -> i32;
    }

    // DPAPI allocates the output with `LocalAlloc`, so it must come back through `LocalFree` and not
    // through Rust's allocator.
    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(handle: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    }

    fn run(protect: bool, input: &[u8]) -> Option<Vec<u8>> {
        let mut source = input.to_vec();
        let mut entropy = ENTROPY.to_vec();
        let data_in = Blob { size: source.len() as u32, data: source.as_mut_ptr() };
        let entropy_in = Blob { size: entropy.len() as u32, data: entropy.as_mut_ptr() };
        let mut out = Blob { size: 0, data: core::ptr::null_mut() };

        // `description` is an optional out parameter on unprotect and an optional in parameter on protect;
        // null in both means DPAPI allocates nothing for it, so there is only ever one buffer to free.
        let ok = unsafe {
            if protect {
                CryptProtectData(
                    &data_in,
                    core::ptr::null(),
                    &entropy_in,
                    core::ptr::null(),
                    core::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            } else {
                CryptUnprotectData(
                    &data_in,
                    core::ptr::null_mut(),
                    &entropy_in,
                    core::ptr::null(),
                    core::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            }
        };

        if ok == 0 || out.data.is_null() {
            return None;
        }
        let bytes = unsafe { core::slice::from_raw_parts(out.data, out.size as usize) }.to_vec();
        unsafe { LocalFree(out.data as *mut core::ffi::c_void) };
        Some(bytes)
    }

    pub fn seal(plaintext: &str) -> Option<Vec<u8>> {
        run(true, plaintext.as_bytes())
    }

    pub fn unseal(sealed: &[u8]) -> Option<Vec<u8>> {
        run(false, sealed)
    }
}

#[cfg(windows)]
pub fn seal(plaintext: &str) -> Option<String> {
    if plaintext.is_empty() {
        return None;
    }
    let blob = dpapi::seal(plaintext)?;
    Some(format!("{TAG}{}", base64::engine::general_purpose::STANDARD.encode(blob)))
}

/// Seal, or `None` when the platform has no DPAPI. The caller writes nothing in that case: a key that cannot
/// be protected is kept in memory rather than committed to disk.
#[cfg(not(windows))]
pub fn seal(_plaintext: &str) -> Option<String> {
    None
}

#[cfg(windows)]
pub fn unseal(sealed: &str) -> Option<String> {
    let encoded = sealed.strip_prefix(TAG)?;
    let blob = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    let plain = dpapi::unseal(&blob)?;
    String::from_utf8(plain).ok()
}

#[cfg(not(windows))]
pub fn unseal(_sealed: &str) -> Option<String> {
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_key() {
        let key = "hc_live_abcdef0123456789";
        let sealed = seal(key).expect("seal");
        assert!(sealed.starts_with(TAG));
        assert!(!sealed.contains(key), "the sealed form must not carry the key");
        assert_eq!(unseal(&sealed).as_deref(), Some(key));
    }

    #[test]
    fn refuses_junk() {
        assert_eq!(unseal("hc_live_abcdef0123456789"), None);
        assert_eq!(unseal("dpapi1:not-base64!!"), None);
        assert_eq!(seal(""), None);
    }

    /// A blob sealed with different entropy is not ours and must not open, which is what keeps another
    /// program running as the same user from decrypting it with a bare DPAPI call.
    #[test]
    fn entropy_is_part_of_the_key() {
        let sealed = seal("hc_live_abcdef0123456789").expect("seal");
        let body = sealed.strip_prefix(TAG).unwrap();
        let mut blob = base64::engine::general_purpose::STANDARD.decode(body).unwrap();
        // Flip one byte of the ciphertext: DPAPI must refuse rather than return garbage.
        let last = blob.len() - 1;
        blob[last] ^= 0xff;
        let tampered = format!("{TAG}{}", base64::engine::general_purpose::STANDARD.encode(blob));
        assert_eq!(unseal(&tampered), None);
    }
}
