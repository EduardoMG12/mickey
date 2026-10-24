//! Who may change a file or folder on Windows (#183): its owner and the
//! rights its access list allows, read straight from Windows. Mickey's logon
//! task runs as administrator only when nobody else can change what it runs.

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

const SE_FILE_OBJECT: u32 = 1;
const OWNER_SECURITY_INFORMATION: u32 = 0x1;
const DACL_SECURITY_INFORMATION: u32 = 0x4;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
/// An entry only handed down to what is created inside, not for this one.
const INHERIT_ONLY_ACE: u8 = 0x08;
/// Rights that let someone change it: write data (or add a file), append
/// (or add a folder), write extended attributes, write attributes, delete,
/// change the permissions, take ownership, generic write and generic all.
const WRITE_LIKE: u32 = 0x0000_0002
    | 0x0000_0004
    | 0x0000_0010
    | 0x0000_0100
    | 0x0001_0000
    | 0x0004_0000
    | 0x0008_0000
    | 0x4000_0000
    | 0x1000_0000;
/// Administrators, SYSTEM and TrustedInstaller, by SID: names change with
/// the language of Windows ("Administradores").
const TRUSTED: [&str; 3] = [
    "S-1-5-32-544",
    "S-1-5-18",
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464",
];
/// CREATOR OWNER: stands for whoever creates something inside, who is then
/// checked as that thing's owner.
const CREATOR_OWNER: &str = "S-1-3-0";

#[repr(C)]
struct AceHeader {
    kind: u8,
    flags: u8,
    size: u16,
}

#[repr(C)]
struct AccessAllowedAce {
    header: AceHeader,
    mask: u32,
    /// The first 4 bytes of the SID, which goes on past the struct.
    sid_start: u32,
}

#[repr(C)]
struct Acl {
    revision: u8,
    sbz1: u8,
    size: u16,
    count: u16,
    sbz2: u16,
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn GetNamedSecurityInfoW(
        name: *const u16,
        kind: u32,
        info: u32,
        owner: *mut *mut c_void,
        group: *mut *mut c_void,
        dacl: *mut *mut Acl,
        sacl: *mut *mut Acl,
        descriptor: *mut *mut c_void,
    ) -> u32;
    fn GetAce(acl: *const Acl, index: u32, ace: *mut *mut c_void) -> i32;
    fn ConvertSidToStringSidW(sid: *const c_void, string: *mut *mut u16) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

/// Whether only Administrators, SYSTEM and TrustedInstaller may change
/// `path`: they own it, and nobody else is allowed a write-like right.
pub fn admin_only(path: &Path) -> io::Result<bool> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut owner = std::ptr::null_mut();
    let mut dacl = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call;
    // the out-pointers are valid locals. On success Windows allocates
    // `descriptor` (freed below with LocalFree), and `owner` and `dacl`
    // point inside it.
    let error = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    let verdict = judge(owner, dacl);
    // SAFETY: `descriptor` came from GetNamedSecurityInfoW, which says to
    // free it with LocalFree; nothing points into it after this.
    unsafe { LocalFree(descriptor) };
    verdict
}

/// The owner must be trusted, and every allow entry that applies to this
/// object with a write-like right must name a trusted SID.
fn judge(owner: *mut c_void, dacl: *mut Acl) -> io::Result<bool> {
    if !TRUSTED.contains(&sid_string(owner)?.as_str()) {
        return Ok(false);
    }
    // No access list at all means everyone may do anything.
    if dacl.is_null() {
        return Ok(false);
    }
    // SAFETY: `dacl` is non-null and points into the live descriptor.
    let count = unsafe { (*dacl).count };
    for index in 0..u32::from(count) {
        let mut ace = std::ptr::null_mut();
        // SAFETY: `index` is below the list's own count; `ace` is a valid
        // out-pointer and, on success, points into the live descriptor.
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let ace = ace as *const AccessAllowedAce;
        // SAFETY: every ACE starts with an AceHeader; mask and sid_start
        // are read only for the allow type, whose layout is this struct.
        let (header, mask) = unsafe { (&(*ace).header, (*ace).mask) };
        if header.kind != ACCESS_ALLOWED_ACE_TYPE
            || header.flags & INHERIT_ONLY_ACE != 0
            || mask & WRITE_LIKE == 0
        {
            continue;
        }
        // SAFETY: the SID starts at `sid_start` and runs inside the ACE.
        let sid = unsafe { &(*ace).sid_start as *const u32 as *const c_void };
        let who = sid_string(sid as *mut c_void)?;
        if !TRUSTED.contains(&who.as_str()) && who != CREATOR_OWNER {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A SID as text ("S-1-5-32-544").
fn sid_string(sid: *mut c_void) -> io::Result<String> {
    let mut text = std::ptr::null_mut();
    // SAFETY: `sid` points to a SID inside the live descriptor; on success
    // Windows allocates `text` (freed below with LocalFree).
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut len = 0;
    // SAFETY: `text` is a NUL-terminated UTF-16 string from Windows.
    while unsafe { *text.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` units of `text` were just read up to the NUL.
    let string = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
    // SAFETY: allocated by ConvertSidToStringSidW, to be freed by LocalFree.
    unsafe { LocalFree(text as *mut c_void) };
    Ok(string)
}

#[cfg(test)]
mod tests {
    /// Run by hand on Windows: `MICKEY_ACL_PATHS='C:\a|C:\b' <test exe>
    /// --ignored --nocapture acl_of`. Prints each path's verdict.
    #[test]
    #[ignore = "reads real files' permissions; run by hand on Windows"]
    fn acl_of() {
        let paths = std::env::var("MICKEY_ACL_PATHS").unwrap_or_default();
        for path in paths.split('|').filter(|p| !p.is_empty()) {
            println!("{path}: {:?}", super::admin_only(std::path::Path::new(path)));
        }
    }
}
