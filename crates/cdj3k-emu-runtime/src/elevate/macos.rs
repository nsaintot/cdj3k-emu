//! Authorization Services. Shows the native admin dialog, which is
//! TouchID- and Apple-Watch-eligible.

use std::io;

type AuthorizationRef = *mut libc::c_void;
type OSStatus = i32;
type AuthorizationFlags = u32;

const K_AUTH_FLAG_DEFAULTS: AuthorizationFlags = 0;
const K_AUTH_FLAG_INTERACTION_ALLOWED: AuthorizationFlags = 1 << 0;
const K_AUTH_FLAG_EXTEND_RIGHTS: AuthorizationFlags = 1 << 1;

#[repr(C)]
struct AuthorizationItem {
    name: *const libc::c_char,
    value_length: libc::size_t,
    value: *mut libc::c_void,
    flags: u32,
}

#[repr(C)]
struct AuthorizationItemSet {
    count: u32,
    items: *mut AuthorizationItem,
}

#[link(name = "Security", kind = "framework")]
extern "C" {
    fn AuthorizationCreate(
        rights: *const AuthorizationItemSet,
        environment: *const AuthorizationItemSet,
        flags: AuthorizationFlags,
        authorization: *mut AuthorizationRef,
    ) -> OSStatus;

    fn AuthorizationCopyRights(
        authorization: AuthorizationRef,
        rights: *const AuthorizationItemSet,
        environment: *const AuthorizationItemSet,
        flags: AuthorizationFlags,
        authorized_rights: *mut *mut AuthorizationItemSet,
    ) -> OSStatus;

    fn AuthorizationExecuteWithPrivileges(
        authorization: AuthorizationRef,
        path_to_tool: *const libc::c_char,
        options: AuthorizationFlags,
        arguments: *const *const libc::c_char,
        communication_pipe: *mut *mut libc::FILE,
    ) -> OSStatus;

    fn AuthorizationFree(authorization: AuthorizationRef, flags: AuthorizationFlags) -> OSStatus;
}

pub fn run_elevated(sh_cmd: &str) -> io::Result<()> {
    use std::ffi::CString;
    use std::ptr;

    let sh_path = CString::new("/bin/sh").unwrap();
    let sh_flag = CString::new("-c").unwrap();
    let cmd = CString::new(sh_cmd).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "shell command contains null byte",
        )
    })?;
    let right_name = CString::new("system.privilege.admin").unwrap();

    unsafe {
        let mut auth: AuthorizationRef = ptr::null_mut();
        let st = AuthorizationCreate(ptr::null(), ptr::null(), K_AUTH_FLAG_DEFAULTS, &mut auth);
        if st != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("AuthorizationCreate failed: {st}"),
            ));
        }

        let mut right_item = AuthorizationItem {
            name: right_name.as_ptr(),
            value_length: 0,
            value: ptr::null_mut(),
            flags: 0,
        };
        let rights = AuthorizationItemSet {
            count: 1,
            items: &mut right_item as *mut _,
        };
        let st = AuthorizationCopyRights(
            auth,
            &rights,
            ptr::null(),
            K_AUTH_FLAG_INTERACTION_ALLOWED | K_AUTH_FLAG_EXTEND_RIGHTS,
            ptr::null_mut(),
        );
        if st != 0 {
            AuthorizationFree(auth, K_AUTH_FLAG_DEFAULTS);
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "admin elevation cancelled or failed",
            ));
        }

        let args: [*const libc::c_char; 3] = [sh_flag.as_ptr(), cmd.as_ptr(), ptr::null()];
        let st = AuthorizationExecuteWithPrivileges(
            auth,
            sh_path.as_ptr(),
            K_AUTH_FLAG_DEFAULTS,
            args.as_ptr(),
            ptr::null_mut(),
        );
        AuthorizationFree(auth, K_AUTH_FLAG_DEFAULTS);

        if st != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("AuthorizationExecuteWithPrivileges failed: {st}"),
            ));
        }
    }
    Ok(())
}
