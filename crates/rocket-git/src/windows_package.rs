//! Windows owns MSIX servicing. Never try to swap an installed package in place.
use crate::{Error, Result};

pub fn identity() -> std::result::Result<bool, u32> {
    #[cfg(windows)]
    {
        static IDENTITY: std::sync::OnceLock<std::result::Result<bool, u32>> =
            std::sync::OnceLock::new();
        *IDENTITY.get_or_init(|| {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn GetCurrentPackageFullName(length: *mut u32, name: *mut u16) -> i32;
            }
            let mut length = 0;
            // No buffer is supplied: Windows only reports whether an identity exists.
            classify(unsafe { GetCurrentPackageFullName(&mut length, std::ptr::null_mut()) } as u32)
        })
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

#[cfg(any(windows, test))]
fn classify(code: u32) -> std::result::Result<bool, u32> {
    match code {
        122 => Ok(true),     // ERROR_INSUFFICIENT_BUFFER: identity buffer needed
        15700 => Ok(false),  // APPMODEL_ERROR_NO_PACKAGE
        other => Err(other), // Unknown identity must not enable self-replacement.
    }
}

pub fn require_unmanaged() -> Result<()> {
    match identity() {
        Ok(false) => Ok(()),
        Ok(true) => Err(Error::InvalidInput("Windows manages this MSIX installation. Update it through Microsoft Store or install a newer signed MSIX package.".into())),
        Err(code) => Err(Error::InvalidInput(format!("Could not determine Windows package identity (error {code}); self-update is disabled."))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_package_identity_fails_closed() {
        assert_eq!(classify(122), Ok(true));
        assert_eq!(classify(15700), Ok(false));
        for code in [0, 5, 87, 123, u32::MAX] {
            assert_eq!(classify(code), Err(code));
        }
    }
}
