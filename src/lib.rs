// Copyright (C) 2015 Florian Wilkens
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
// associated documentation files (the "Software"), to deal in the Software without restriction,
// including without limitation the rights to use, copy, modify, merge, publish, distribute,
// sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all copies or substantial
// portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
// NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
// NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES
// OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
// CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

// Crate dependencies
extern crate libc;
extern crate pam_sys as pam;
extern crate users;

// Modules
mod ffi;
mod simple;

// Re-Exports
pub use simple::*;

// Usings
use pam::{PamConversation, PamFlag, PamHandle, PamReturnCode};

/// Main struct to authenticate a user
/// Currently closes the session on drop() but this might change!
pub struct Authenticator<'a> {
    /// Flag indicating whether the Authenticator should close the session on drop
    pub close_on_drop:  bool,
    handle:             *mut PamHandle,
    credentials:        Box<[&'a str; 2]>,
    is_authenticated:   bool,
    has_open_session:   bool,
    last_code:          PamReturnCode
}

impl <'a> Authenticator<'a> {
    /// Creates a new Authenticator with a given service name
    pub fn new(service: &str) -> Option<Authenticator> {
        use std::ffi::CString;
        use std::ptr;

        let creds = Box::new([""; 2]);
        let conv = PamConversation {
            conv:       Some(ffi::converse),
            data_ptr:   creds.as_ptr() as *mut ::libc::c_void
        };
        let mut handle: *mut PamHandle = ptr::null_mut();

        match unsafe { pam::start(CString::new(service).unwrap().as_ptr(), ptr::null(), &conv, &mut handle) } {
            PamReturnCode::SUCCESS => Some(Authenticator {
                close_on_drop:      true,
                handle:             handle,
                credentials:        creds,
                is_authenticated:   false,
                has_open_session:   false,
                last_code:          PamReturnCode::SUCCESS
            }),
            _   => None
        }
    }

    /// Set the credentials which should be used in the authentication process.
    /// Currently only username/password combinations are supported
    pub fn set_credentials(&mut self, user: &'a str, password: &'a str) {
        self.credentials[0] = user;
        self.credentials[1] = password;
    }

    /// Perform the authentication with the provided credentials
    pub fn authenticate(&mut self) -> Result<(), PamReturnCode> {
        let success = PamReturnCode::SUCCESS;

        self.last_code = unsafe { pam::authenticate(self.handle, PamFlag::NONE) };
        if self.last_code != success {
            // No need to reset here
            return Err(self.last_code);
        }
        self.is_authenticated = true;

        self.last_code = unsafe { pam::acct_mgmt(self.handle, PamFlag::NONE) };
        if self.last_code != success {
            // Probably not strictly neccessary but better be sure
            return self.reset();
        }

        self.last_code = unsafe { pam::setcred(self.handle, PamFlag::ESTABLISH_CRED) };
        if self.last_code != success {
            return self.reset();
        }
        Ok(())
    }

    /// Open a session for a previously authenticated user and
    /// initialize the environment appropriately (in PAM and regular enviroment variables).
    pub fn open_session(&mut self) -> Result<(), PamReturnCode> {
        if !self.is_authenticated {
            //TODO: is this the right return code?
            return Err(PamReturnCode::PERM_DENIED);
        }

        self.last_code = unsafe { pam::open_session(self.handle, PamFlag::NONE) };
        if self.last_code != PamReturnCode::SUCCESS {
            return self.reset();
        }

        self.has_open_session = true;
        self.initialize_environment()
    }

    // Initialize the client environment with common variables.
    // Currently always called from Authenticator.open_session()
    fn initialize_environment(&self) -> Result<(), PamReturnCode> {
        let user = users::get_user_by_name(self.credentials[0])
            .expect(&format!("Could not get user by name: {:?}", self.credentials[0]));

        self.set_env("USER", &user.name)
            .and(self.set_env("LOGNAME", &user.name))
            .and(self.set_env("HOME", &user.home_dir))
            .and(self.set_env("PWD", &user.home_dir))
            .and(self.set_env("SHELL", &user.shell))
            // Taken from https://github.com/gsingh93/display-manager/blob/master/pam.c
            // Should be a better way to get this. Revisit later.
            .and(self.set_env("PATH", "$PATH:/usr/local/sbin:/usr/local/bin:/usr/bin"))
    }

    // Utility function to set an environment variable in PAM and the process
    fn set_env(&self, key: &str, value: &str) -> Result<(), PamReturnCode> {
        use std::env;
        use std::ffi::CString;

        // Set regular environment variable
        env::set_var(key, value);

        // Set pam environment variable
        let name_value = CString::new(format!("{}={}", key, value)).unwrap();
        match unsafe { pam::putenv(self.handle, name_value.as_ptr()) } {
            PamReturnCode::SUCCESS  => Ok(()),
            code                    => Err(code)
        }
    }

    // Utility function to reset the pam handle in case of intermediate errors
    fn reset(&mut self) -> Result<(), PamReturnCode> {
        unsafe {
            pam::setcred(self.handle, PamFlag::DELETE_CRED);
        }
        self.is_authenticated = false;
        Err(self.last_code)
    }
}

impl <'a> Drop for Authenticator<'a> {
    fn drop(&mut self) {
        unsafe {
            if self.has_open_session && self.close_on_drop {
                pam::close_session(self.handle, PamFlag::NONE);
            }
            let code = pam::setcred(self.handle, PamFlag::DELETE_CRED);
            pam::end(self.handle, code);
        }
    }
}
