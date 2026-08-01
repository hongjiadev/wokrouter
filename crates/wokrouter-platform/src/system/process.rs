use std::{num::NonZeroU32, path::Path};

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_regular_file_without_following_symlinks(path: &Path) -> Option<std::fs::File> {
    use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .ok()?;
    file.metadata().ok()?.is_file().then_some(file)
}

#[cfg(windows)]
pub(super) fn process_executable_matches(process_id: NonZeroU32, candidate: &Path) -> bool {
    use std::{
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
        },
        ptr,
    };

    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
            OPEN_EXISTING,
        },
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        },
    };

    fn open_without_following_reparse(path: &Path) -> Option<OwnedHandle> {
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            None
        } else {
            Some(unsafe { OwnedHandle::from_raw_handle(handle) })
        }
    }

    fn file_identity(handle: &OwnedHandle) -> Option<(u32, u32, u32)> {
        let mut information = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
        if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut information) } == 0
            || information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0
        {
            return None;
        }
        Some((
            information.dwVolumeSerialNumber,
            information.nFileIndexHigh,
            information.nFileIndexLow,
        ))
    }

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id.get()) };
    if process.is_null() {
        return false;
    }
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    let mut image = vec![0_u16; 32_768];
    let mut image_length = image.len() as u32;
    if unsafe {
        QueryFullProcessImageNameW(
            process.as_raw_handle(),
            0,
            image.as_mut_ptr(),
            &mut image_length,
        )
    } == 0
        || image_length == 0
        || image_length as usize > image.len()
    {
        return false;
    }
    let Ok(image) = String::from_utf16(&image[..image_length as usize]) else {
        return false;
    };
    let Some(image) = open_without_following_reparse(Path::new(&image)) else {
        return false;
    };
    let Some(candidate) = open_without_following_reparse(candidate) else {
        return false;
    };
    file_identity(&image).is_some_and(|identity| Some(identity) == file_identity(&candidate))
}

#[cfg(target_os = "linux")]
pub(super) fn process_executable_matches(process_id: NonZeroU32, candidate: &Path) -> bool {
    use std::{fs::File, os::unix::fs::MetadataExt};

    let Ok(process_image) = File::open(format!("/proc/{}/exe", process_id.get())) else {
        return false;
    };
    let Some(candidate) = open_regular_file_without_following_symlinks(candidate) else {
        return false;
    };
    let (Ok(process_image), Ok(candidate)) = (process_image.metadata(), candidate.metadata())
    else {
        return false;
    };
    process_image.is_file()
        && candidate.is_file()
        && process_image.dev() == candidate.dev()
        && process_image.ino() == candidate.ino()
}

#[cfg(target_os = "macos")]
pub(super) fn process_executable_matches(process_id: NonZeroU32, candidate: &Path) -> bool {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    let mut image = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let image_length = unsafe {
        libc::proc_pidpath(
            process_id.get() as libc::c_int,
            image.as_mut_ptr().cast(),
            image.len() as u32,
        )
    };
    if image_length <= 0 || image_length as usize > image.len() {
        return false;
    }
    let Some(terminator) = image.iter().position(|byte| *byte == 0) else {
        return false;
    };
    if terminator == 0 || terminator > image_length as usize {
        return false;
    }
    let image = Path::new(OsStr::from_bytes(&image[..terminator]));
    paths_match_file_identity_without_following_symlinks(image, candidate)
}

#[cfg(target_os = "macos")]
fn paths_match_file_identity_without_following_symlinks(image: &Path, candidate: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Some(process_image) = open_regular_file_without_following_symlinks(image) else {
        return false;
    };
    let Some(candidate) = open_regular_file_without_following_symlinks(candidate) else {
        return false;
    };
    let (Ok(process_image), Ok(candidate)) = (process_image.metadata(), candidate.metadata())
    else {
        return false;
    };
    process_image.is_file()
        && candidate.is_file()
        && process_image.dev() == candidate.dev()
        && process_image.ino() == candidate.ino()
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use std::os::unix::fs::symlink;

    use super::paths_match_file_identity_without_following_symlinks;

    #[test]
    fn file_identity_comparison_rejects_symlinks_on_both_sides() {
        let temporary = tempfile::tempdir().expect("create temporary directory");
        let executable = temporary.path().join("wokcore");
        std::fs::write(&executable, b"wokcore").expect("write executable");
        let alias = temporary.path().join("wokcore-alias");
        symlink(&executable, &alias).expect("create symlink");

        assert!(!paths_match_file_identity_without_following_symlinks(
            &alias,
            &executable
        ));
        assert!(!paths_match_file_identity_without_following_symlinks(
            &executable,
            &alias
        ));
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(super) fn process_executable_matches(_process_id: NonZeroU32, _candidate: &Path) -> bool {
    false
}
