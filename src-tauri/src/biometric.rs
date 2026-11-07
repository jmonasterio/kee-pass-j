//! Quick unlock: after one unlock with the master password, the password is kept so that the
//! next unlock only needs Windows Hello (Windows) or Touch ID (macOS).
//!
//! # Windows
//!
//! One Windows Hello key credential per user ([`CREDENTIAL_NAME`]) holds an RSA-2048 key in the
//! TPM (or in software when there is no TPM). Signing data with it always shows the Windows Hello
//! prompt. To protect a password, a random 32-byte challenge is signed and SHA-256 of the
//! signature becomes an AES-256-GCM key. RSA PKCS#1 v1.5 signatures are deterministic, so signing
//! the stored challenge again yields the same key. This is the scheme KeePassXC uses. The
//! encrypted password lives in `data_dir()/quickunlock/<hex SHA-256 of the database path>`:
//!
//! | bytes | content                                                         |
//! |-------|-----------------------------------------------------------------|
//! | 4     | magic `OLQU`                                                    |
//! | 1     | format version, 1                                               |
//! | 32    | challenge                                                       |
//! | 12    | AES-GCM nonce                                                   |
//! | rest  | AES-256-GCM ciphertext + 16-byte tag; AAD = the first 37 bytes |
//!
//! Without a successful Windows Hello verification the signature, and so the key, cannot be
//! produced. The file is useless on another machine or account.
//!
//! # macOS
//!
//! The password is a generic password in the user's login Keychain (service [`MAC_SERVICE`],
//! account = absolute database path), and [`retrieve`] reads it only after
//! `LAContext.evaluatePolicy(.deviceOwnerAuthenticationWithBiometrics)` succeeds.
//!
//! Trade-off: a Keychain item bound to biometry (`SecAccessControl` with `.biometryCurrentSet`)
//! must live in the data-protection keychain, which requires a signed app with a
//! keychain-access-groups entitlement. Our app is ad-hoc signed, so the Touch ID check is enforced
//! by this code and not by the Keychain. The login Keychain still encrypts the item at rest with
//! the user's login password and its ACL only lets this app read it without a Keychain prompt, but
//! code running as the user that macOS lets read the item could get the password without Touch
//! ID. After an app update the code signature changes and macOS may ask once to allow access.

use std::path::Path;

use zeroize::Zeroizing;

/// The biometric mechanism quick unlock uses on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    #[cfg_attr(not(windows), allow(dead_code))]
    WindowsHello,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    TouchId,
}

/// Name of the Windows Hello key credential shared by all databases.
#[cfg(windows)]
const CREDENTIAL_NAME: &str = "com.logonpro.ourlogins.quickunlock";

/// Keychain service of the stored passwords on macOS.
#[cfg(target_os = "macos")]
const MAC_SERVICE: &str = "com.logonpro.ourlogins.quickunlock";

/// Hardware present and the user has it set up. Cheap enough to call on page load.
pub fn available() -> Option<Kind> {
    imp::available()
}

/// True when a protected password is stored for this database path.
pub fn has(db_path: &Path) -> bool {
    imp::has(db_path)
}

/// Stores `password` for `db_path`, protected so it can only be read back after a successful
/// Windows Hello / Touch ID check. May show the OS prompt (Windows Hello key creation). `parent`
/// is the native window handle to own the prompt (HWND as isize on Windows; ignored on macOS).
pub fn store(db_path: &Path, password: &str, parent: Option<isize>) -> Result<(), String> {
    imp::store(db_path, password, parent)
}

/// Shows the OS prompt with `reason` and returns the stored password. Windows Hello has no
/// caller-supplied text, so `reason` is only shown on macOS.
pub fn retrieve(db_path: &Path, reason: &str, parent: Option<isize>) -> Result<Zeroizing<String>, String> {
    imp::retrieve(db_path, reason, parent)
}

/// Deletes what is stored for this database (no prompt). Missing = Ok.
pub fn forget(db_path: &Path) {
    imp::forget(db_path)
}

/// The absolute database path, lowercased on Windows where paths compare case-insensitively.
/// Not canonicalized, for the same reason as `vault::path_hash`: canonicalization depends on
/// whether the file exists and adds `\\?\` on Windows.
#[cfg(any(windows, target_os = "macos"))]
fn path_key(db_path: &Path) -> String {
    let abs = std::path::absolute(db_path).unwrap_or_else(|_| db_path.to_path_buf());
    let text = abs.to_string_lossy().into_owned();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// Converts decrypted bytes to the password, keeping every copy zeroized.
#[cfg(any(windows, target_os = "macos"))]
fn into_password(mut bytes: Zeroizing<Vec<u8>>) -> Result<Zeroizing<String>, String> {
    match String::from_utf8(std::mem::take(&mut *bytes)) {
        Ok(s) => Ok(Zeroizing::new(s)),
        Err(e) => {
            drop(Zeroizing::new(e.into_bytes()));
            Err("The stored quick unlock password is damaged.".to_owned())
        }
    }
}

/// File format and encryption of the Windows store. Pure functions, tested without Windows Hello.
/// SHA-256 and AES-256-GCM come from Windows' own crypto library (BCrypt).
#[cfg(windows)]
mod sealed {
    use windows::Win32::Security::Cryptography::{
        BCryptDecrypt, BCryptDestroyKey, BCryptEncrypt, BCryptGenerateSymmetricKey, BCryptHash, BCRYPT_AES_GCM_ALG_HANDLE,
        BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO, BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION, BCRYPT_FLAGS, BCRYPT_KEY_HANDLE,
        BCRYPT_SHA256_ALG_HANDLE,
    };
    use zeroize::Zeroizing;

    const MAGIC: &[u8; 4] = b"OLQU";
    const VERSION: u8 = 1;
    pub const CHALLENGE_LEN: usize = 32;
    const NONCE_LEN: usize = 12;
    /// Magic + version + challenge; authenticated as AAD.
    const AAD_LEN: usize = MAGIC.len() + 1 + CHALLENGE_LEN;
    const HEADER_LEN: usize = AAD_LEN + NONCE_LEN;
    const TAG_LEN: usize = 16;

    fn sha256(data: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        let status = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) };
        assert!(status.is_ok(), "SHA-256 failed: {status:?}");
        out
    }

    /// An AES-256-GCM key handle, destroyed on drop.
    struct GcmKey(BCRYPT_KEY_HANDLE);

    impl GcmKey {
        fn new(key: &[u8; 32]) -> GcmKey {
            let mut handle = BCRYPT_KEY_HANDLE::default();
            let status = unsafe { BCryptGenerateSymmetricKey(BCRYPT_AES_GCM_ALG_HANDLE, &mut handle, None, key, 0) };
            assert!(status.is_ok(), "AES key setup failed: {status:?}");
            GcmKey(handle)
        }
    }

    impl Drop for GcmKey {
        fn drop(&mut self) {
            unsafe {
                let _ = BCryptDestroyKey(self.0);
            }
        }
    }

    fn mode_info(nonce: &[u8], aad: &[u8], tag: &mut [u8; TAG_LEN]) -> BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
        BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
            cbSize: std::mem::size_of::<BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO>() as u32,
            dwInfoVersion: BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION,
            pbNonce: nonce.as_ptr() as *mut u8,
            cbNonce: nonce.len() as u32,
            pbAuthData: aad.as_ptr() as *mut u8,
            cbAuthData: aad.len() as u32,
            pbTag: tag.as_mut_ptr(),
            cbTag: TAG_LEN as u32,
            ..Default::default()
        }
    }

    /// File name for a database: lowercase hex SHA-256 of its path key.
    pub fn file_name(path_key: &str) -> String {
        hex(&sha256(path_key.as_bytes()))
    }

    /// AES-256 key derived from a Windows Hello signature.
    pub fn key_from_signature(signature: &[u8]) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(sha256(signature))
    }

    /// Encrypts `plaintext` and returns the complete file content:
    /// magic, version, challenge, nonce, ciphertext, tag.
    pub fn seal(key: &[u8; 32], challenge: &[u8; CHALLENGE_LEN], nonce: &[u8; NONCE_LEN], plaintext: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + plaintext.len() + TAG_LEN);
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(challenge);
        let mut tag = [0u8; TAG_LEN];
        let mut ciphertext = vec![0u8; plaintext.len()];
        let info = mode_info(nonce, &out, &mut tag);
        let gcm = GcmKey::new(key);
        let mut written = 0u32;
        let status = unsafe {
            BCryptEncrypt(
                gcm.0,
                Some(plaintext),
                Some(&info as *const _ as *const core::ffi::c_void),
                None,
                Some(&mut ciphertext),
                &mut written,
                BCRYPT_FLAGS(0),
            )
        };
        assert!(status.is_ok(), "AES-GCM encryption failed: {status:?}");
        out.extend_from_slice(nonce);
        out.extend_from_slice(&ciphertext[..written as usize]);
        out.extend_from_slice(&tag);
        out
    }

    /// The challenge stored in a file, after checking magic, version and minimum length.
    pub fn challenge(file: &[u8]) -> Result<&[u8], String> {
        if file.len() < HEADER_LEN + TAG_LEN || &file[..MAGIC.len()] != MAGIC {
            return Err("The stored quick unlock data is damaged.".to_owned());
        }
        if file[MAGIC.len()] != VERSION {
            return Err("The stored quick unlock data was written by a newer version of Our Logins.".to_owned());
        }
        Ok(&file[MAGIC.len() + 1..AAD_LEN])
    }

    /// Decrypts a file written by [`seal`]. Fails when the key differs, e.g. after the Windows
    /// Hello key was recreated.
    pub fn open(key: &[u8; 32], file: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
        challenge(file)?;
        let (body, tag) = file[HEADER_LEN..].split_at(file.len() - HEADER_LEN - TAG_LEN);
        let mut tag: [u8; TAG_LEN] = tag.try_into().expect("split at TAG_LEN");
        let info = mode_info(&file[AAD_LEN..HEADER_LEN], &file[..AAD_LEN], &mut tag);
        let gcm = GcmKey::new(key);
        let mut plaintext = Zeroizing::new(vec![0u8; body.len()]);
        let mut written = 0u32;
        let status = unsafe {
            BCryptDecrypt(
                gcm.0,
                Some(body),
                Some(&info as *const _ as *const core::ffi::c_void),
                None,
                Some(&mut plaintext[..]),
                &mut written,
                BCRYPT_FLAGS(0),
            )
        };
        if !status.is_ok() {
            return Err("The stored quick unlock password no longer matches your Windows Hello key. \
                        Unlock with the master password to set up quick unlock again."
                .to_owned());
        }
        plaintext.truncate(written as usize);
        Ok(plaintext)
    }

    pub fn random<const N: usize>() -> Result<[u8; N], String> {
        let mut buf = [0u8; N];
        getrandom::fill(&mut buf).map_err(|e| format!("Could not get random bytes: {e}"))?;
        Ok(buf)
    }

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write;
        bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use windows::core::{s, w, IInspectable, Interface, GUID, HRESULT, HSTRING};
    use windows::Security::Credentials::{
        KeyCredential, KeyCredentialCreationOption, KeyCredentialManager, KeyCredentialOperationResult,
        KeyCredentialRetrievalResult, KeyCredentialStatus,
    };
    use windows::Security::Cryptography::CryptographicBuffer;
    use windows::Storage::Streams::IBuffer;
    use windows::UI::WindowId;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow};
    use windows_future::IAsyncOperation;
    use zeroize::{Zeroize, Zeroizing};

    use super::{sealed, Kind, CREDENTIAL_NAME};

    const CANCELLED: &str = "Windows Hello was cancelled.";

    pub fn available() -> Option<Kind> {
        let supported = KeyCredentialManager::IsSupportedAsync().and_then(|op| op.join()).unwrap_or(false);
        supported.then_some(Kind::WindowsHello)
    }

    pub fn has(db_path: &Path) -> bool {
        file_path(db_path).is_file()
    }

    pub fn store(db_path: &Path, password: &str, parent: Option<isize>) -> Result<(), String> {
        let window = window_id(parent);
        let credential = open_or_create(window)?;
        let challenge = sealed::random::<{ sealed::CHALLENGE_LEN }>()?;
        let key = sealed::key_from_signature(&sign(&credential, &challenge, window)?);
        let file = sealed::seal(&key, &challenge, &sealed::random()?, password.as_bytes());

        let path = file_path(db_path);
        let dir = path.parent().expect("quick unlock file has a parent directory");
        std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &file).map_err(|e| format!("Could not write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("Could not write {}: {e}", path.display())
        })
    }

    pub fn retrieve(db_path: &Path, _reason: &str, parent: Option<isize>) -> Result<Zeroizing<String>, String> {
        let path = file_path(db_path);
        let file = match std::fs::read(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err("Quick unlock is not set up for this database.".to_owned())
            }
            Err(e) => return Err(format!("Could not read {}: {e}", path.display())),
        };
        let challenge = sealed::challenge(&file)?;
        let credential = open()?;
        let window = window_id(parent);
        let key = sealed::key_from_signature(&sign(&credential, challenge, window)?);
        super::into_password(sealed::open(&key, &file)?)
    }

    pub fn forget(db_path: &Path) {
        let _ = std::fs::remove_file(file_path(db_path));
    }

    fn file_path(db_path: &Path) -> PathBuf {
        ourlogins::settings::data_dir()
            .join("quickunlock")
            .join(sealed::file_name(&super::path_key(db_path)))
    }

    fn status_error(status: KeyCredentialStatus) -> String {
        match status {
            KeyCredentialStatus::UserCanceled | KeyCredentialStatus::UserPrefersPassword => CANCELLED.to_owned(),
            KeyCredentialStatus::NotFound => "The Windows Hello key for Our Logins no longer exists. \
                 Unlock with the master password to set up quick unlock again."
                .to_owned(),
            KeyCredentialStatus::SecurityDeviceLocked => "Windows Hello is locked after too many attempts. \
                 Sign in to Windows with your PIN, then try again."
                .to_owned(),
            _ => "Windows Hello reported an unknown error.".to_owned(),
        }
    }

    fn api_error(e: windows::core::Error) -> String {
        // NTE_PERM: OpenAsync and RequestCreateAsync fail with it when the Windows account has no
        // Windows Hello PIN (observed with `dsregcmd /status` showing NgcSet : NO).
        if e.code() == HRESULT(0x8009_0010_u32 as i32) {
            return "Windows Hello is not set up. Add a PIN in Settings > Accounts > Sign-in options.".to_owned();
        }
        format!("Windows Hello failed: {} ({})", e.message(), e.code())
    }

    /// Opens the key credential; `NotFound` becomes an error.
    fn open() -> Result<KeyCredential, String> {
        let result = KeyCredentialManager::OpenAsync(&HSTRING::from(CREDENTIAL_NAME))
            .and_then(|op| op.join())
            .map_err(api_error)?;
        credential_of(&result)
    }

    /// Opens the key credential, creating it (Windows Hello prompt) when it does not exist yet.
    fn open_or_create(window: Option<WindowId>) -> Result<KeyCredential, String> {
        let name = HSTRING::from(CREDENTIAL_NAME);
        let opened = KeyCredentialManager::OpenAsync(&name).and_then(|op| op.join()).map_err(api_error)?;
        if opened.Status().map_err(api_error)? != KeyCredentialStatus::NotFound {
            return credential_of(&opened);
        }
        let op = match window.and_then(|id| create_for_window(id, &name)) {
            Some(op) => op,
            None => {
                foreground_prompt_soon();
                KeyCredentialManager::RequestCreateAsync(&name, KeyCredentialCreationOption::FailIfExists)
                    .map_err(api_error)?
            }
        };
        let created = op.join().map_err(api_error)?;
        if created.Status().map_err(api_error)? == KeyCredentialStatus::CredentialAlreadyExists {
            // Created by another call between OpenAsync and RequestCreateAsync.
            return open();
        }
        credential_of(&created)
    }

    fn credential_of(result: &KeyCredentialRetrievalResult) -> Result<KeyCredential, String> {
        match result.Status().map_err(api_error)? {
            KeyCredentialStatus::Success => result.Credential().map_err(api_error),
            status => Err(status_error(status)),
        }
    }

    /// Signs `data` with the key credential (Windows Hello prompt) and returns the signature.
    fn sign(credential: &KeyCredential, data: &[u8], window: Option<WindowId>) -> Result<Zeroizing<Vec<u8>>, String> {
        let buffer = CryptographicBuffer::CreateFromByteArray(data).map_err(api_error)?;
        let op = match window.and_then(|id| sign_for_window(credential, id, &buffer)) {
            Some(op) => op,
            None => {
                foreground_prompt_soon();
                credential.RequestSignAsync(&buffer).map_err(api_error)?
            }
        };
        let result = op.join().map_err(api_error)?;
        match result.Status().map_err(api_error)? {
            KeyCredentialStatus::Success => {}
            status => return Err(status_error(status)),
        }
        let signature = result.Result().map_err(api_error)?;
        let mut bytes = windows::core::Array::<u8>::new();
        CryptographicBuffer::CopyToByteArray(&signature, &mut bytes).map_err(api_error)?;
        let out = Zeroizing::new(bytes.to_vec());
        bytes.zeroize();
        Ok(out)
    }

    // Windows 11 24H2+ variants that take the owner window, so the prompt is modal to it and
    // comes to the foreground. The `windows` crate (0.62) does not project them yet; the IIDs
    // and method order come from Windows.Security.Credentials.winmd of build 26300.

    /// `IKeyCredentialWithWindow`: `RequestSignForWindowAsync(WindowId, IBuffer)`.
    const IID_KEY_CREDENTIAL_WITH_WINDOW: GUID = GUID::from_u128(0x050deb3b_b19c_4635_9df6_5650d66c62b1);
    /// `IKeyCredentialManagerCreateWithWindowStatics`:
    /// `RequestCreateForWindowAsync(WindowId, String, KeyCredentialCreationOption)`.
    const IID_KEY_CREDENTIAL_MANAGER_CREATE_WITH_WINDOW_STATICS: GUID =
        GUID::from_u128(0x30b1b9c9_61ef_43e8_88ac_cc433b38d1a6);

    #[repr(C)]
    struct KeyCredentialWithWindowVtbl {
        base: windows::core::IInspectable_Vtbl,
        request_sign_for_window_async:
            unsafe extern "system" fn(*mut c_void, WindowId, *mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct KeyCredentialManagerCreateWithWindowStaticsVtbl {
        base: windows::core::IInspectable_Vtbl,
        request_create_for_window_async: unsafe extern "system" fn(
            *mut c_void,
            WindowId,
            *mut c_void,
            KeyCredentialCreationOption,
            *mut *mut c_void,
        ) -> HRESULT,
    }

    /// QueryInterface for an interface the `windows` crate does not know.
    fn query(object: &impl Interface, iid: &GUID) -> Option<IInspectable> {
        let mut raw = std::ptr::null_mut();
        // SAFETY: `query` writes an owned, AddRef'd pointer on success; `from_raw` takes that reference.
        unsafe { object.query(iid, &mut raw).ok().ok().map(|()| IInspectable::from_raw(raw)) }
    }

    /// `None` when this Windows has no `RequestSignForWindowAsync` or the call fails to start.
    fn sign_for_window(
        credential: &KeyCredential,
        window: WindowId,
        data: &IBuffer,
    ) -> Option<IAsyncOperation<KeyCredentialOperationResult>> {
        let iface = query(credential, &IID_KEY_CREDENTIAL_WITH_WINDOW)?;
        let mut op = std::ptr::null_mut();
        // SAFETY: `iface` implements IKeyCredentialWithWindow, whose vtable is laid out as declared;
        // on success `op` is an owned IAsyncOperation<KeyCredentialOperationResult>.
        unsafe {
            let vtbl = &**(iface.as_raw() as *const *const KeyCredentialWithWindowVtbl);
            (vtbl.request_sign_for_window_async)(iface.as_raw(), window, data.as_raw(), &mut op).ok().ok()?;
            Some(IAsyncOperation::from_raw(op))
        }
    }

    /// `None` when this Windows has no `RequestCreateForWindowAsync` or the call fails to start.
    fn create_for_window(window: WindowId, name: &HSTRING) -> Option<IAsyncOperation<KeyCredentialRetrievalResult>> {
        let factory = windows::core::factory::<KeyCredentialManager, IInspectable>().ok()?;
        let statics = query(&factory, &IID_KEY_CREDENTIAL_MANAGER_CREATE_WITH_WINDOW_STATICS)?;
        let mut op = std::ptr::null_mut();
        // SAFETY: as in `sign_for_window`; an HSTRING is passed by its handle, borrowed for the call.
        unsafe {
            let vtbl = &**(statics.as_raw() as *const *const KeyCredentialManagerCreateWithWindowStaticsVtbl);
            let name_abi: *mut c_void = std::mem::transmute_copy(name);
            (vtbl.request_create_for_window_async)(
                statics.as_raw(),
                window,
                name_abi,
                KeyCredentialCreationOption::FailIfExists,
                &mut op,
            )
            .ok()
            .ok()?;
            Some(IAsyncOperation::from_raw(op))
        }
    }

    /// `GetWindowIdFromWindow` (windows.ui.interop.h) from its API set; `None` without a parent
    /// window or on Windows versions that lack it.
    fn window_id(parent: Option<isize>) -> Option<WindowId> {
        type GetWindowIdFromWindow = unsafe extern "system" fn(HWND, *mut WindowId) -> HRESULT;
        let hwnd = HWND(parent.filter(|&h| h != 0)? as *mut c_void);
        // SAFETY: the export has the signature of GetWindowIdFromWindow in windows.ui.interop.h.
        // The module stays loaded for the life of the process.
        unsafe {
            let module =
                LoadLibraryExW(w!("ext-ms-win-windowing-external-l1-1-0.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
            let proc = GetProcAddress(module, s!("GetWindowIdFromWindow"))?;
            let get: GetWindowIdFromWindow = std::mem::transmute(proc);
            let mut id = WindowId::default();
            get(hwnd, &mut id).ok().ok()?;
            Some(id)
        }
    }

    /// Without an owner window the Windows Hello prompt can open behind the app. Bring it to the
    /// front once it appears (KeePassXC does the same). The app is in the foreground when the
    /// user starts an unlock, so Windows allows this.
    fn foreground_prompt_soon() {
        std::thread::spawn(|| {
            for _ in 0..50 {
                std::thread::sleep(Duration::from_millis(100));
                // SAFETY: plain Win32 calls with valid arguments.
                if let Ok(hwnd) = unsafe { FindWindowW(w!("Credential Dialog Xaml Host"), None) } {
                    let _ = unsafe { SetForegroundWindow(hwnd) };
                    return;
                }
            }
        });
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::Path;
    use std::sync::mpsc;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAError, LAPolicy};
    use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
    use security_framework::passwords::{delete_generic_password, generic_password, set_generic_password, PasswordOptions};
    use zeroize::Zeroizing;

    use super::{Kind, MAC_SERVICE};

    const CANCELLED: &str = "Touch ID was cancelled.";
    /// `errSecUserCanceled`: the user dismissed the Keychain access prompt.
    const ERR_SEC_USER_CANCELED: i32 = -128;
    /// `errSecItemNotFound`.
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    pub fn available() -> Option<Kind> {
        // SAFETY: LAContext is usable from any thread.
        let ok = unsafe { LAContext::new().canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics) };
        ok.is_ok().then_some(Kind::TouchId)
    }

    pub fn has(db_path: &Path) -> bool {
        // Attributes only: reading the password data could show a Keychain prompt.
        ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(MAC_SERVICE)
            .account(&super::path_key(db_path))
            .load_attributes(true)
            .limit(Limit::Max(1))
            .search()
            .is_ok_and(|found| !found.is_empty())
    }

    pub fn store(db_path: &Path, password: &str, _parent: Option<isize>) -> Result<(), String> {
        set_generic_password(MAC_SERVICE, &super::path_key(db_path), password.as_bytes())
            .map_err(|e| format!("Could not save the password in the Keychain: {e}"))
    }

    pub fn retrieve(db_path: &Path, reason: &str, _parent: Option<isize>) -> Result<Zeroizing<String>, String> {
        if !has(db_path) {
            return Err("Quick unlock is not set up for this database.".to_owned());
        }
        verify(reason)?;
        let account = super::path_key(db_path);
        match generic_password(PasswordOptions::new_generic_password(MAC_SERVICE, &account)) {
            Ok(bytes) => super::into_password(Zeroizing::new(bytes)),
            Err(e) if e.code() == ERR_SEC_USER_CANCELED => Err(CANCELLED.to_owned()),
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => {
                Err("Quick unlock is not set up for this database.".to_owned())
            }
            Err(e) => Err(format!("Could not read the password from the Keychain: {e}")),
        }
    }

    pub fn forget(db_path: &Path) {
        let _ = delete_generic_password(MAC_SERVICE, &super::path_key(db_path));
    }

    /// Shows the Touch ID prompt and blocks until it is answered.
    fn verify(reason: &str) -> Result<(), String> {
        // evaluatePolicy throws NSInvalidArgumentException on an empty reason.
        let reason = if reason.trim().is_empty() { "unlock your database" } else { reason };
        let (tx, rx) = mpsc::channel::<Result<(), isize>>();
        let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
            let outcome = if success.as_bool() {
                Ok(())
            } else {
                // SAFETY: LocalAuthentication passes nil or a valid NSError.
                Err(unsafe { error.as_ref() }.map_or(LAError::AuthenticationFailed.0, |e| e.code()))
            };
            let _ = tx.send(outcome);
        });
        // SAFETY: the reply block only sends on a channel, which is thread-safe. `context` stays
        // alive until the reply arrives; releasing it earlier would cancel the evaluation.
        let context = unsafe { LAContext::new() };
        unsafe {
            context.evaluatePolicy_localizedReason_reply(
                LAPolicy::DeviceOwnerAuthenticationWithBiometrics,
                &NSString::from_str(reason),
                &reply,
            );
        }
        let outcome = rx.recv().map_err(|_| "Touch ID did not answer.".to_owned())?;
        drop(context);
        outcome.map_err(|code| match LAError(code) {
            LAError::UserCancel | LAError::SystemCancel | LAError::AppCancel | LAError::UserFallback => {
                CANCELLED.to_owned()
            }
            LAError::BiometryLockout => {
                "Touch ID is locked after too many attempts. Unlock your Mac with its password, then try again."
                    .to_owned()
            }
            LAError::BiometryNotAvailable | LAError::BiometryNotEnrolled | LAError::PasscodeNotSet => {
                "Touch ID is not set up on this Mac.".to_owned()
            }
            LAError::AuthenticationFailed => "Touch ID did not recognize your fingerprint.".to_owned(),
            _ => format!("Touch ID failed (error {code})."),
        })
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use std::path::Path;

    use zeroize::Zeroizing;

    use super::Kind;

    pub fn available() -> Option<Kind> {
        None
    }

    pub fn has(_db_path: &Path) -> bool {
        false
    }

    pub fn store(_db_path: &Path, _password: &str, _parent: Option<isize>) -> Result<(), String> {
        Err("not supported".to_owned())
    }

    pub fn retrieve(_db_path: &Path, _reason: &str, _parent: Option<isize>) -> Result<Zeroizing<String>, String> {
        Err("not supported".to_owned())
    }

    pub fn forget(_db_path: &Path) {}
}

#[cfg(all(test, windows))]
mod tests {
    use super::sealed;

    const KEY: [u8; 32] = [7; 32];
    const CHALLENGE: [u8; 32] = [9; 32];
    const NONCE: [u8; 12] = [3; 12];

    #[test]
    fn file_name_is_hex_sha256_of_lowercased_absolute_path() {
        let a = sealed::file_name(&super::path_key(std::path::Path::new(r"C:\Users\Me\Vault.kdbx")));
        let b = sealed::file_name(&super::path_key(std::path::Path::new(r"c:\users\me\vault.KDBX")));
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
        // SHA-256("abc"), FIPS 180-2 test vector.
        assert_eq!(sealed::file_name("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn seal_open_round_trip() {
        let file = sealed::seal(&KEY, &CHALLENGE, &NONCE, "pässwörd".as_bytes());
        assert_eq!(&file[..4], b"OLQU");
        assert_eq!(sealed::challenge(&file).unwrap(), &CHALLENGE);
        assert_eq!(&*sealed::open(&KEY, &file).unwrap(), "pässwörd".as_bytes());
        let password = super::into_password(sealed::open(&KEY, &file).unwrap()).unwrap();
        assert_eq!(password.as_str(), "pässwörd");
    }

    /// Files written by the earlier `aes-gcm` build must still open. The expected bytes come
    /// from an independent AES-256-GCM (Python `cryptography`) over the same inputs.
    #[test]
    fn seal_matches_independent_aes_gcm() {
        let expected = "4f4c5155010909090909090909090909090909090909090909090909090909090909090909\
                        030303030303030303030303553d0770295f9df408240053f17d0e39c42a016e64af2856d8d4";
        let file = sealed::seal(&KEY, &CHALLENGE, &NONCE, "pässwörd".as_bytes());
        let hex: String = file.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, expected);
    }

    #[test]
    fn wrong_key_or_tampering_fails() {
        let file = sealed::seal(&KEY, &CHALLENGE, &NONCE, b"secret");
        assert!(sealed::open(&[8; 32], &file).is_err());
        for i in [0, 4, 5, 40, file.len() - 1] {
            let mut bad = file.clone();
            bad[i] ^= 1;
            assert!(sealed::open(&KEY, &bad).is_err(), "byte {i}");
        }
        assert!(sealed::open(&KEY, &file[..60]).is_err());
    }

    #[test]
    fn key_from_signature_is_sha256() {
        let key = sealed::key_from_signature(b"abc");
        assert_eq!(key[..4], [0xba, 0x78, 0x16, 0xbf]);
    }
}
