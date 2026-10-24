//! Who may change a file or folder on Windows (#183): its owner and the
//! rights its access list allows, read straight from Windows. Mickey's logon
//! task runs as administrator only when nobody else can change what it runs.

use std::ffi::OsString;
use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const SE_FILE_OBJECT: u32 = 1;
const OWNER_SECURITY_INFORMATION: u32 = 0x1;
const DACL_SECURITY_INFORMATION: u32 = 0x4;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
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
    fn GetCurrentProcess() -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
    fn GetTokenInformation(
        token: *mut c_void,
        class: u32,
        info: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetProcessMitigationPolicy(policy: u32, buffer: *const c_void, length: usize) -> i32;
}

/// PROCESS_MITIGATION_POLICY's ProcessRedirectionTrustPolicy.
const REDIRECTION_TRUST_POLICY: u32 = 16;
/// Its EnforceRedirectionTrust flag.
const ENFORCE_REDIRECTION_TRUST: u32 = 0x1;

/// From now on this process does not follow junctions made by users
/// without administrator rights: opening a path through one fails. Mickey
/// run as administrator writes into the user's folders (its control file,
/// received files, config.toml); without this, a program of the user
/// could turn one of those folders into a junction to a system folder and
/// have Mickey overwrite system files. An error where Windows is too old
/// to know the policy.
pub fn refuse_user_junctions() -> io::Result<()> {
    let flags = ENFORCE_REDIRECTION_TRUST;
    // SAFETY: the policy's buffer is one u32 of flags, and its size is
    // given; Windows only reads it during the call.
    let ok = unsafe {
        SetProcessMitigationPolicy(
            REDIRECTION_TRUST_POLICY,
            (&raw const flags).cast(),
            size_of::<u32>(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    JUNCTIONS_REFUSED.store(true, Ordering::Relaxed);
    Ok(())
}

static JUNCTIONS_REFUSED: AtomicBool = AtomicBool::new(false);

/// Whether `refuse_user_junctions` worked in this process.
pub fn user_junctions_refused() -> bool {
    JUNCTIONS_REFUSED.load(Ordering::Relaxed)
}

const TOKEN_QUERY: u32 = 0x0008;
/// TOKEN_INFORMATION_CLASS's TokenElevation: a u32, non-zero if elevated.
const TOKEN_ELEVATION: u32 = 20;

/// Whether this process runs as administrator (elevated by UAC, or by a
/// task's highest run level).
pub fn elevated() -> bool {
    let mut token = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no
    // closing; `token` is a valid out-pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let mut elevation = 0u32;
    let mut returned = 0u32;
    // SAFETY: `token` was just opened with TOKEN_QUERY; `elevation` is a
    // u32, the size TokenElevation writes, and its length is given.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TOKEN_ELEVATION,
            (&raw mut elevation).cast(),
            size_of::<u32>() as u32,
            &mut returned,
        )
    } != 0;
    // SAFETY: `token` was opened above and is closed once.
    unsafe { CloseHandle(token) };
    ok && elevation != 0
}

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

/// FOLDERID_ProgramFiles, {905E63B6-C1BF-494E-B29C-65B732D3D21A}.
const PROGRAM_FILES: Guid = Guid {
    data1: 0x905e_63b6,
    data2: 0xc1bf,
    data3: 0x494e,
    data4: [0xb2, 0x9c, 0x65, 0xb7, 0x32, 0xd3, 0xd2, 0x1a],
};

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHGetKnownFolderPath(
        id: *const Guid,
        flags: u32,
        token: *mut c_void,
        path: *mut *mut u16,
    ) -> i32;
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoTaskMemFree(memory: *mut c_void);
}

/// The Program Files folder, from Windows itself: the `ProgramFiles`
/// variable could be set by any program in the user's environment.
pub fn program_files() -> io::Result<PathBuf> {
    let mut path = std::ptr::null_mut();
    // SAFETY: the GUID is a valid constant; `path` is a valid out-pointer.
    // Windows allocates the string even on failure; CoTaskMemFree below.
    let result =
        unsafe { SHGetKnownFolderPath(&PROGRAM_FILES, 0, std::ptr::null_mut(), &mut path) };
    let folder = if result >= 0 && !path.is_null() {
        let mut len = 0;
        // SAFETY: on success `path` is a NUL-terminated UTF-16 string.
        while unsafe { *path.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: `len` units of `path` were just read up to the NUL.
        let wide = unsafe { std::slice::from_raw_parts(path, len) };
        Ok(PathBuf::from(OsString::from_wide(wide)))
    } else {
        Err(io::Error::other(format!(
            "SHGetKnownFolderPath failed: {result:#x}"
        )))
    };
    // SAFETY: allocated by SHGetKnownFolderPath (null is allowed), freed once.
    unsafe { CoTaskMemFree(path as *mut c_void) };
    folder
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
        // SAFETY: every ACE starts with an AceHeader.
        let header = unsafe { &(*ace).header };
        match header.kind {
            // A deny entry only takes rights away: ignoring it is the
            // cautious side.
            ACCESS_DENIED_ACE_TYPE => continue,
            ACCESS_ALLOWED_ACE_TYPE => {}
            // Any other kind (conditional, per-object...) may allow
            // writing in ways not read here: not trusted.
            _ => return Ok(false),
        }
        // SAFETY: an allow entry has this struct's layout, so its mask is
        // inside the ACE.
        let mask = unsafe { (*ace).mask };
        if header.flags & INHERIT_ONLY_ACE != 0 || mask & WRITE_LIKE == 0 {
            continue;
        }
        // SAFETY: the SID starts at `sid_start` and runs on inside the ACE;
        // a raw pointer to the field (not a reference to its 4 bytes) may
        // be read past them.
        let sid: *const c_void = unsafe { (&raw const (*ace).sid_start).cast() };
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
            println!(
                "{path}: {:?}",
                super::admin_only(std::path::Path::new(path))
            );
        }
    }

    /// Run by hand on Windows, as administrator: `MICKEY_JUNCTION_FILE` is
    /// a file path through a junction a user without administrator rights
    /// made. Writing it must fail once the policy is on.
    #[test]
    #[ignore = "needs a junction made by a standard user; run by hand on Windows"]
    fn a_user_junction_is_not_followed() {
        let through = std::env::var("MICKEY_JUNCTION_FILE").unwrap();
        println!("policy: {:?}", super::refuse_user_junctions());
        println!("write: {:?}", std::fs::write(&through, "x"));
    }

    #[test]
    fn program_files_is_found() {
        let folder = super::program_files().unwrap();
        assert!(
            folder.is_absolute() && folder.is_dir(),
            "{}",
            folder.display()
        );
    }
}
