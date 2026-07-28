use std::{
    ffi::c_void,
    fs::File,
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        fs::MetadataExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::Path,
    ptr,
};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_SUCCESS, GENERIC_ALL, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
        LocalFree,
    },
    Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        Authorization::{
            ConvertStringSidToSidW, EXPLICIT_ACCESS_W, GetSecurityInfo, NO_MULTIPLE_TRUSTEE,
            SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW, SetSecurityInfo, TRUSTEE_IS_SID,
            TRUSTEE_IS_USER, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
        },
        CreateWellKnownSid, DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation,
        GetSecurityDescriptorControl, GetTokenInformation, INHERIT_ONLY_ACE, INHERITED_ACE,
        NO_INHERITANCE, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, SECURITY_MAX_SID_SIZE,
        SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_QUERY, TOKEN_USER, TokenUser,
        WinBuiltinAdministratorsSid, WinLocalSystemSid,
    },
    Storage::FileSystem::{
        CreateFileW, DELETE, FILE_ALL_ACCESS, FILE_APPEND_DATA, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_DELETE_CHILD, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
        FILE_WRITE_DATA, FILE_WRITE_EA, OPEN_EXISTING, READ_CONTROL, WRITE_DAC, WRITE_OWNER,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

#[derive(Clone, Copy)]
pub(crate) enum PrivatePathKind {
    File,
    Directory,
}

pub(crate) fn secure_private_path(path: &Path, kind: PrivatePathKind) -> io::Result<()> {
    let file = open_security_handle(path, kind, READ_CONTROL | WRITE_DAC | WRITE_OWNER)?;
    if !matches_kind_without_reparse(&file, kind) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe Windows path type",
        ));
    }
    apply_private_owner_and_dacl(&file, kind)?;
    private_owned_by_current_user_and_system(&file, kind)
        .then_some(())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unsafe Windows path permissions",
            )
        })
}

pub(crate) fn private_path_owned_by_current_user_and_system(
    path: &Path,
    kind: PrivatePathKind,
) -> bool {
    let Ok(file) = open_security_handle(path, kind, READ_CONTROL) else {
        return false;
    };
    matches_kind_without_reparse(&file, kind)
        && private_owned_by_current_user_and_system(&file, kind)
}

pub(crate) fn private_owned_by_current_user_and_system(file: &File, kind: PrivatePathKind) -> bool {
    owned_by_current_user(file) && private_dacl_allows_only_user_and_system(file, kind)
}

pub(crate) fn path_executable_is_not_untrusted_writable(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let Ok(file) = open_security_handle(path, PrivatePathKind::File, READ_CONTROL) else {
        return false;
    };
    if !matches_kind_without_reparse(&file, PrivatePathKind::File)
        || !dacl_has_no_untrusted_mutator(&file, PrivatePathKind::File)
    {
        return false;
    }
    let mut parent = path.parent();
    while let Some(directory) = parent {
        let Ok(handle) = open_security_handle(directory, PrivatePathKind::Directory, READ_CONTROL)
        else {
            return false;
        };
        if !matches_kind_without_reparse(&handle, PrivatePathKind::Directory)
            || !dacl_has_no_untrusted_mutator(&handle, PrivatePathKind::Directory)
        {
            return false;
        }
        parent = directory.parent();
    }
    true
}

fn open_security_handle(
    path: &Path,
    kind: PrivatePathKind,
    desired_access: u32,
) -> io::Result<File> {
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let flags = FILE_FLAG_OPEN_REPARSE_POINT
        | match kind {
            PrivatePathKind::File => 0,
            PrivatePathKind::Directory => FILE_FLAG_BACKUP_SEMANTICS,
        };
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            desired_access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            flags,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn matches_kind_without_reparse(file: &File, kind: PrivatePathKind) -> bool {
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    let expected_kind = match kind {
        PrivatePathKind::File => metadata.is_file(),
        PrivatePathKind::Directory => metadata.is_dir(),
    };
    expected_kind && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

fn apply_private_owner_and_dacl(file: &File, kind: PrivatePathKind) -> io::Result<()> {
    let user = current_user_sid().ok_or_else(io::Error::last_os_error)?;
    let system = local_system_sid().ok_or_else(io::Error::last_os_error)?;
    let inheritance = match kind {
        PrivatePathKind::File => NO_INHERITANCE,
        PrivatePathKind::Directory => SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    };
    let entries = [
        explicit_access(user.as_ptr(), TRUSTEE_IS_USER, inheritance),
        explicit_access(system.as_ptr(), TRUSTEE_IS_WELL_KNOWN_GROUP, inheritance),
    ];
    apply_explicit_dacl(file, &entries, Some(user.as_ptr()))
}

fn apply_explicit_dacl(
    file: &File,
    entries: &[EXPLICIT_ACCESS_W],
    owner: Option<PSID>,
) -> io::Result<()> {
    let mut acl: *mut ACL = ptr::null_mut();
    let status = unsafe {
        SetEntriesInAclW(
            entries.len() as u32,
            entries.as_ptr(),
            ptr::null(),
            &mut acl,
        )
    };
    let acl = LocalAllocation(acl.cast());
    if status != ERROR_SUCCESS || acl.0.is_null() {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let security_information = DACL_SECURITY_INFORMATION
        | PROTECTED_DACL_SECURITY_INFORMATION
        | owner
            .is_some()
            .then_some(OWNER_SECURITY_INFORMATION)
            .unwrap_or_default();
    let status = unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            security_information,
            owner.unwrap_or(ptr::null_mut()),
            ptr::null_mut(),
            acl.0.cast(),
            ptr::null(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

fn explicit_access(sid: PSID, trustee_type: i32, inheritance: u32) -> EXPLICIT_ACCESS_W {
    EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        grfInheritance: inheritance,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: trustee_type,
            ptstrName: sid.cast(),
        },
    }
}

fn owned_by_current_user(file: &File) -> bool {
    let Some(user) = current_user_sid() else {
        return false;
    };
    let Some(security) = security_descriptor(file, OWNER_SECURITY_INFORMATION) else {
        return false;
    };
    !security.owner.is_null() && unsafe { EqualSid(security.owner, user.as_ptr()) != 0 }
}

fn private_dacl_allows_only_user_and_system(file: &File, kind: PrivatePathKind) -> bool {
    let Some(user) = current_user_sid() else {
        return false;
    };
    let Some(system) = local_system_sid() else {
        return false;
    };
    let Some(security) = security_descriptor(file, DACL_SECURITY_INFORMATION) else {
        return false;
    };
    if security.dacl.is_null() {
        return false;
    }
    let mut control = 0;
    let mut revision = 0;
    if unsafe { GetSecurityDescriptorControl(security.descriptor.0, &mut control, &mut revision) }
        == 0
        || control & SE_DACL_PROTECTED == 0
    {
        return false;
    }
    let mut information = ACL_SIZE_INFORMATION::default();
    if unsafe {
        GetAclInformation(
            security.dacl,
            (&raw mut information).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
        || information.AceCount != 2
    {
        return false;
    }
    let expected_flags = match kind {
        PrivatePathKind::File => NO_INHERITANCE,
        PrivatePathKind::Directory => SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    } as u8;
    let mut user_found = false;
    let mut system_found = false;
    for index in 0..information.AceCount {
        let mut raw_ace: *mut c_void = ptr::null_mut();
        if unsafe { GetAce(security.dacl, index, &mut raw_ace) } == 0 || raw_ace.is_null() {
            return false;
        }
        let ace = unsafe { &*raw_ace.cast::<ACCESS_ALLOWED_ACE>() };
        if ace.Header.AceType != 0
            || ace.Header.AceFlags & INHERITED_ACE as u8 != 0
            || ace.Header.AceFlags != expected_flags
            || ace.Mask != FILE_ALL_ACCESS
        {
            return false;
        }
        let sid = (&raw const ace.SidStart).cast_mut().cast::<c_void>();
        if unsafe { EqualSid(sid, user.as_ptr()) } != 0 {
            if user_found {
                return false;
            }
            user_found = true;
        } else if unsafe { EqualSid(sid, system.as_ptr()) } != 0 {
            if system_found {
                return false;
            }
            system_found = true;
        } else {
            return false;
        }
    }
    user_found && system_found
}

fn dacl_has_no_untrusted_mutator(file: &File, kind: PrivatePathKind) -> bool {
    let Some(user) = current_user_sid() else {
        return false;
    };
    let Some(system) = local_system_sid() else {
        return false;
    };
    let Some(administrators) = well_known_sid(WinBuiltinAdministratorsSid) else {
        return false;
    };
    let Some(trusted_installer) = trusted_installer_sid() else {
        return false;
    };
    let Some(security) =
        security_descriptor(file, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION)
    else {
        return false;
    };
    let trusted = [
        user.as_ptr(),
        system.as_ptr(),
        administrators.as_ptr(),
        trusted_installer.as_ptr(),
    ];
    if security.owner.is_null()
        || !trusted
            .iter()
            .any(|sid| unsafe { EqualSid(security.owner, *sid) } != 0)
        || security.dacl.is_null()
    {
        return false;
    }
    let mut information = ACL_SIZE_INFORMATION::default();
    if unsafe {
        GetAclInformation(
            security.dacl,
            (&raw mut information).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return false;
    }
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const OTHER_ACCESS_ALLOWED_ACE_TYPES: [u8; 4] = [4, 5, 9, 11];
    let unsafe_write_mask = match kind {
        PrivatePathKind::File => {
            FILE_WRITE_DATA
                | FILE_APPEND_DATA
                | FILE_WRITE_EA
                | FILE_WRITE_ATTRIBUTES
                | DELETE
                | WRITE_DAC
                | WRITE_OWNER
                | GENERIC_WRITE
                | GENERIC_ALL
        }
        PrivatePathKind::Directory => {
            FILE_WRITE_DATA
                | FILE_DELETE_CHILD
                | DELETE
                | WRITE_DAC
                | WRITE_OWNER
                | GENERIC_WRITE
                | GENERIC_ALL
        }
    };
    for index in 0..information.AceCount {
        let mut raw_ace: *mut c_void = ptr::null_mut();
        if unsafe { GetAce(security.dacl, index, &mut raw_ace) } == 0 || raw_ace.is_null() {
            return false;
        }
        let ace = unsafe { &*raw_ace.cast::<ACCESS_ALLOWED_ACE>() };
        if ace.Header.AceFlags & INHERIT_ONLY_ACE as u8 != 0 {
            continue;
        }
        if OTHER_ACCESS_ALLOWED_ACE_TYPES.contains(&ace.Header.AceType) {
            return false;
        }
        if ace.Header.AceType != ACCESS_ALLOWED_ACE_TYPE || ace.Mask & unsafe_write_mask == 0 {
            continue;
        }
        let sid = (&raw const ace.SidStart).cast_mut().cast::<c_void>();
        if !trusted
            .iter()
            .any(|trusted_sid| unsafe { EqualSid(sid, *trusted_sid) } != 0)
        {
            return false;
        }
    }
    true
}

fn security_descriptor(file: &File, information: u32) -> Option<SecurityDescriptor> {
    let mut owner: PSID = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            information,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS || descriptor.is_null() {
        return None;
    }
    Some(SecurityDescriptor {
        descriptor: LocalAllocation(descriptor.cast()),
        owner,
        dacl,
    })
}

fn current_user_sid() -> Option<SidBuffer> {
    let mut token: HANDLE = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let token = Token(token);
    let mut required = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut required);
    }
    if required == 0 {
        return None;
    }
    let mut storage = vec![0_usize; (required as usize).div_ceil(size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            storage.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return None;
    }
    Some(SidBuffer {
        storage,
        token_user: true,
    })
}

fn local_system_sid() -> Option<SidBuffer> {
    well_known_sid(WinLocalSystemSid)
}

fn well_known_sid(kind: i32) -> Option<SidBuffer> {
    let mut storage = vec![0_usize; (SECURITY_MAX_SID_SIZE as usize).div_ceil(size_of::<usize>())];
    let mut size = SECURITY_MAX_SID_SIZE;
    if unsafe {
        CreateWellKnownSid(
            kind,
            ptr::null_mut(),
            storage.as_mut_ptr().cast(),
            &mut size,
        )
    } == 0
    {
        return None;
    }
    Some(SidBuffer {
        storage,
        token_user: false,
    })
}

fn trusted_installer_sid() -> Option<OwnedSid> {
    let text = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464\0"
        .encode_utf16()
        .collect::<Vec<_>>();
    let mut sid: PSID = ptr::null_mut();
    if unsafe { ConvertStringSidToSidW(text.as_ptr(), &mut sid) } == 0 || sid.is_null() {
        return None;
    }
    Some(OwnedSid(LocalAllocation(sid)))
}

struct SidBuffer {
    storage: Vec<usize>,
    token_user: bool,
}

impl SidBuffer {
    fn as_ptr(&self) -> PSID {
        if self.token_user {
            let token_user = unsafe { &*self.storage.as_ptr().cast::<TOKEN_USER>() };
            token_user.User.Sid
        } else {
            self.storage.as_ptr().cast_mut().cast()
        }
    }
}

struct OwnedSid(LocalAllocation);

impl OwnedSid {
    fn as_ptr(&self) -> PSID {
        self.0.0
    }
}

struct SecurityDescriptor {
    descriptor: LocalAllocation,
    owner: PSID,
    dacl: *mut ACL,
}

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}

struct Token(HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::{TempDir, tempdir, tempdir_in};
    use windows_sys::Win32::{
        Security::{
            Authorization::{TRUSTEE_IS_USER, TRUSTEE_IS_WELL_KNOWN_GROUP},
            NO_INHERITANCE, WinBuiltinUsersSid,
        },
        Storage::FileSystem::{
            FILE_ALL_ACCESS, FILE_GENERIC_READ, FILE_GENERIC_WRITE, READ_CONTROL,
        },
    };

    use super::{
        PrivatePathKind, apply_explicit_dacl, current_user_sid, explicit_access, local_system_sid,
        open_security_handle, owned_by_current_user, path_executable_is_not_untrusted_writable,
        private_dacl_allows_only_user_and_system, private_path_owned_by_current_user_and_system,
        secure_private_path, well_known_sid,
    };

    #[test]
    fn private_acl_is_applied_to_files_and_directories() {
        let fixture = tempdir().unwrap();
        let directory = fixture.path().join("private");
        let file = directory.join("record.json");
        fs::create_dir(&directory).unwrap();
        fs::write(&file, b"{}").unwrap();
        assert!(!private_path_owned_by_current_user_and_system(
            &file,
            PrivatePathKind::File
        ));

        secure_private_path(&directory, PrivatePathKind::Directory).unwrap();
        secure_private_path(&file, PrivatePathKind::File).unwrap();

        assert!(private_path_owned_by_current_user_and_system(
            &directory,
            PrivatePathKind::Directory
        ));
        assert!(private_path_owned_by_current_user_and_system(
            &file,
            PrivatePathKind::File
        ));
        let secured_directory =
            open_security_handle(&directory, PrivatePathKind::Directory, READ_CONTROL).unwrap();
        assert!(owned_by_current_user(&secured_directory));
        assert!(private_dacl_allows_only_user_and_system(
            &secured_directory,
            PrivatePathKind::Directory
        ));
        let secured_file =
            open_security_handle(&file, PrivatePathKind::File, READ_CONTROL).unwrap();
        assert!(owned_by_current_user(&secured_file));
        assert!(private_dacl_allows_only_user_and_system(
            &secured_file,
            PrivatePathKind::File
        ));
    }

    #[test]
    fn reparse_points_are_rejected_when_the_platform_allows_creating_one() {
        use std::os::windows::fs::symlink_file;

        let fixture = tempdir().unwrap();
        let target = fixture.path().join("target");
        let link = fixture.path().join("link");
        fs::write(&target, b"target").unwrap();
        if symlink_file(&target, &link).is_err() {
            return;
        }

        assert!(!private_path_owned_by_current_user_and_system(
            &link,
            PrivatePathKind::File
        ));
        assert!(secure_private_path(&link, PrivatePathKind::File).is_err());
    }

    #[test]
    fn path_executables_allow_public_read_but_reject_public_write() {
        let fixture = trusted_path_fixture();
        let executable = fixture.path().join("wokcore.exe");
        fs::write(&executable, b"executable").unwrap();

        apply_users_access(&executable, FILE_GENERIC_READ);
        assert!(path_executable_is_not_untrusted_writable(&executable));

        apply_users_access(&executable, FILE_GENERIC_WRITE);
        assert!(!path_executable_is_not_untrusted_writable(&executable));
    }

    #[test]
    fn path_executables_reject_an_untrusted_writable_parent() {
        use windows_sys::Win32::Storage::FileSystem::FILE_WRITE_DATA;

        let fixture = trusted_path_fixture();
        let directory = fixture.path().join("bin");
        let executable = directory.join("wokcore.exe");
        fs::create_dir(&directory).unwrap();
        fs::write(&executable, b"executable").unwrap();

        apply_users_access(&executable, FILE_GENERIC_READ);
        assert!(path_executable_is_not_untrusted_writable(&executable));
        apply_users_access_with_kind(&directory, PrivatePathKind::Directory, FILE_WRITE_DATA);
        assert!(!path_executable_is_not_untrusted_writable(&executable));
    }

    fn trusted_path_fixture() -> TempDir {
        let local_app_data = std::env::var_os("LOCALAPPDATA").unwrap();
        tempdir_in(local_app_data).unwrap()
    }

    fn apply_users_access(path: &Path, users_access: u32) {
        apply_users_access_with_kind(path, PrivatePathKind::File, users_access);
    }

    fn apply_users_access_with_kind(path: &Path, kind: PrivatePathKind, users_access: u32) {
        use windows_sys::Win32::Storage::FileSystem::{READ_CONTROL, WRITE_DAC};

        let file = open_security_handle(path, kind, READ_CONTROL | WRITE_DAC).unwrap();
        let user = current_user_sid().unwrap();
        let system = local_system_sid().unwrap();
        let users = well_known_sid(WinBuiltinUsersSid).unwrap();
        let entries = [
            explicit_access(user.as_ptr(), TRUSTEE_IS_USER, NO_INHERITANCE),
            explicit_access(system.as_ptr(), TRUSTEE_IS_WELL_KNOWN_GROUP, NO_INHERITANCE),
            super::EXPLICIT_ACCESS_W {
                grfAccessPermissions: users_access,
                ..explicit_access(users.as_ptr(), TRUSTEE_IS_WELL_KNOWN_GROUP, NO_INHERITANCE)
            },
        ];
        assert_eq!(entries[0].grfAccessPermissions, FILE_ALL_ACCESS);
        apply_explicit_dacl(&file, &entries, None).unwrap();
    }
}
