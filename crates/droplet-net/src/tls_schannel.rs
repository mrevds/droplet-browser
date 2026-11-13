//! Windows Schannel TLS client built on raw SSPI/CryptoAPI FFI.
//!
//! This module is compiled only on Windows. It provides the same blocking
//! `Read`/`Write`/`close` surface as the Linux OpenSSL backend, with system
//! certificate-store trust validation and SNI derived from the target host.
//!
//! Live handshake testing requires a Windows host; the rest of the workspace is
//! developed and tested on Linux.

use std::ffi::c_void;
use std::io::{ErrorKind, Read, Write};
use std::mem;
use std::net::TcpStream;
use std::os::raw::c_char;
use std::ptr;
use std::slice;
use std::time::{Duration, Instant};

use crate::dns::strip_ipv6_brackets;
use crate::{NetError, HANDSHAKE_TIMEOUT, READ_TIMEOUT};

type SecStatus = i32;

// ---------------------------------------------------------------------------
// SSPI constants
// ---------------------------------------------------------------------------

const SEC_E_OK: SecStatus = 0;
const SEC_I_CONTINUE_NEEDED: SecStatus = 0x0009_0312;
const SEC_I_INCOMPLETE_CREDENTIALS: SecStatus = 0x0009_0320;
const SEC_I_RENEGOTIATE: SecStatus = 0x0009_0321;
const SEC_I_CONTEXT_EXPIRED: SecStatus = 0x0009_0317;
const SEC_E_INCOMPLETE_MESSAGE: SecStatus = 0x8009_0318u32 as i32;
const SEC_E_INVALID_TOKEN: SecStatus = 0x8009_0308u32 as i32;
const SEC_E_UNTRUSTED_ROOT: SecStatus = 0x8009_0325u32 as i32;
const SEC_E_WRONG_PRINCIPAL: SecStatus = 0x8009_0322u32 as i32;
const SEC_E_CERT_EXPIRED: SecStatus = 0x8009_0328u32 as i32;
const SEC_E_CERT_UNKNOWN: SecStatus = 0x8009_0327u32 as i32;
const SEC_E_ILLEGAL_MESSAGE: SecStatus = 0x8009_0326u32 as i32;
const SEC_E_MESSAGE_ALTERED: SecStatus = 0x8009_030Fu32 as i32;
const SEC_E_OUT_OF_SEQUENCE: SecStatus = 0x8009_0310u32 as i32;
const SEC_E_DECRYPT_FAILURE: SecStatus = 0x8009_0330u32 as i32;
const SEC_E_ENCRYPT_FAILURE: SecStatus = 0x8009_0329u32 as i32;
const SEC_E_ALGORITHM_MISMATCH: SecStatus = 0x8009_0331u32 as i32;

const SECPKG_CRED_OUTBOUND: u32 = 2;
const SECURITY_NATIVE_DREP: u32 = 0x0000_0010;

const SECBUFFER_VERSION: u32 = 0;
const SECBUFFER_EMPTY: u32 = 0;
const SECBUFFER_DATA: u32 = 1;
const SECBUFFER_TOKEN: u32 = 2;
const SECBUFFER_EXTRA: u32 = 5;
const SECBUFFER_STREAM_TRAILER: u32 = 6;
const SECBUFFER_STREAM_HEADER: u32 = 7;

const SECPKG_ATTR_STREAM_SIZES: u32 = 4;
const SECPKG_ATTR_REMOTE_CERT_CONTEXT: u32 = 0x53;

const ISC_REQ_REPLAY_DETECT: u32 = 0x0000_0004;
const ISC_REQ_SEQUENCE_DETECT: u32 = 0x0000_0008;
const ISC_REQ_CONFIDENTIALITY: u32 = 0x0000_0010;
const ISC_REQ_ALLOCATE_MEMORY: u32 = 0x0000_0100;
const ISC_REQ_EXTENDED_ERROR: u32 = 0x0000_4000;
const ISC_REQ_STREAM: u32 = 0x0000_8000;

const SCHANNEL_CRED_VERSION: u32 = 4;
const SCH_CRED_NO_DEFAULT_CREDS: u32 = 0x0000_0010;
const SCH_CRED_AUTO_CRED_VALIDATION: u32 = 0x0000_0020;
const SCH_USE_STRONG_CRYPTO: u32 = 0x0040_0000;

const CERT_CHAIN_POLICY_SSL: usize = 4;
const AUTHTYPE_SERVER: u32 = 2;

const MAX_TLS_RECORD: usize = 16 * 1024 + 2 * 1024;
const MAX_HANDSHAKE_BYTES: usize = 256 * 1024;
const MAX_PENDING_CIPHERTEXT: usize = 256 * 1024;

// ---------------------------------------------------------------------------
// SSPI structures
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SecHandle {
    dw_lower: usize,
    dw_upper: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TimeStamp {
    low_part: u32,
    high_part: i32,
}

#[repr(C)]
struct SecBuffer {
    cb_buffer: u32,
    buffer_type: u32,
    pv_buffer: *mut c_void,
}

#[repr(C)]
struct SecBufferDesc {
    ul_version: u32,
    c_buffers: u32,
    p_buffers: *mut SecBuffer,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SecPkgContextStreamSizes {
    cb_header: u32,
    cb_trailer: u32,
    cb_maximum_message: u32,
    c_buffers: u32,
    cb_block_size: u32,
}

/// `SCHANNEL_CRED` from `schannel.h`; size 80 bytes on 64-bit Windows.
#[repr(C)]
#[allow(dead_code)]
struct SchannelCred {
    dw_version: u32,
    c_creds: u32,
    pa_cred: *mut *mut c_void,
    h_root_store: *mut c_void,
    c_mappers: u32,
    aph_mappers: *mut *mut c_void,
    c_supported_algs: u32,
    palg_supported_algs: *mut u32,
    grbit_enabled_protocols: u32,
    dw_minimum_cipher_strength: u32,
    dw_maximum_cipher_strength: u32,
    dw_session_lifespan: u32,
    dw_flags: u32,
    dw_cred_format: u32,
}

#[repr(C)]
struct CertEnhKeyUsage {
    c_usage_identifier: u32,
    rgpsz_usage_identifier: *mut *mut c_char,
}

#[repr(C)]
struct CertUsageMatch {
    dw_type: u32,
    usage: CertEnhKeyUsage,
}

/// `CERT_CHAIN_PARA`; size 96 bytes on 64-bit Windows.
#[repr(C)]
#[allow(dead_code)]
struct CertChainPara {
    cb_size: u32,
    requested_usage: CertUsageMatch,
    requested_issuance_policy: CertUsageMatch,
    dw_url_retrieval_timeout: u32,
    f_check_revocation_freshness_time: i32,
    dw_revocation_freshness_time: u32,
    pft_cache_resync: *mut c_void,
    p_strong_sign_para: *mut c_void,
    dw_strong_sign_flags: u32,
}

#[repr(C)]
struct CertChainPolicyPara {
    cb_size: u32,
    dw_flags: u32,
    pv_extra_policy_para: *mut c_void,
}

#[repr(C)]
struct CertChainPolicyStatus {
    cb_size: u32,
    dw_error: u32,
    l_chain_index: i32,
    l_element_index: i32,
    pv_extra_policy_status: *mut c_void,
}

/// `HTTPSPolicyCallbackData` (`SSL_EXTRA`) used by `CERT_CHAIN_POLICY_SSL`.
#[repr(C)]
struct HttpsPolicyCallbackData {
    cb_size: u32,
    dw_auth_type: u32,
    fdw_checks: u32,
    pwsz_server_name: *const u16,
}

// ---------------------------------------------------------------------------
// SSPI / CryptoAPI imports
// ---------------------------------------------------------------------------

#[link(name = "secur32")]
extern "system" {
    fn AcquireCredentialsHandleW(
        psz_principal: *const u16,
        psz_package: *const u16,
        f_credential_use: u32,
        pv_logon_id: *mut c_void,
        p_auth_data: *mut c_void,
        p_get_key_fn: *mut c_void,
        pv_get_key_argument: *mut c_void,
        ph_credential: *mut SecHandle,
        pts_expiry: *mut TimeStamp,
    ) -> SecStatus;

    fn InitializeSecurityContextW(
        ph_credential: *mut SecHandle,
        ph_context: *mut SecHandle,
        psz_target_name: *const u16,
        f_context_req: u32,
        reserved1: u32,
        target_data_rep: u32,
        p_input: *mut SecBufferDesc,
        reserved2: u32,
        ph_new_context: *mut SecHandle,
        p_output: *mut SecBufferDesc,
        pf_context_attr: *mut u32,
        pts_expiry: *mut TimeStamp,
    ) -> SecStatus;

    fn QueryContextAttributesW(
        ph_context: *mut SecHandle,
        ul_attribute: u32,
        p_buffer: *mut c_void,
    ) -> SecStatus;

    fn EncryptMessage(
        ph_context: *mut SecHandle,
        f_qop: u32,
        p_message: *mut SecBufferDesc,
        message_seq_no: u32,
    ) -> SecStatus;

    fn DecryptMessage(
        ph_context: *mut SecHandle,
        p_message: *mut SecBufferDesc,
        message_seq_no: u32,
        pf_qop: *mut u32,
    ) -> SecStatus;

    fn FreeContextBuffer(pv_context_buffer: *mut c_void) -> SecStatus;
    fn FreeCredentialsHandle(ph_credential: *mut SecHandle) -> SecStatus;
    fn DeleteSecurityContext(ph_context: *mut SecHandle) -> SecStatus;
}

#[link(name = "crypt32")]
extern "system" {
    fn CertGetCertificateChain(
        h_chain_engine: *mut c_void,
        p_cert_context: *const c_void,
        p_time: *mut c_void,
        h_additional_store: *mut c_void,
        p_chain_para: *const CertChainPara,
        dw_flags: u32,
        pv_reserved: *mut c_void,
        pp_chain_context: *mut *mut c_void,
    ) -> i32;

    fn CertVerifyCertificateChainPolicy(
        psz_policy_oid: *const c_char,
        p_chain_context: *const c_void,
        p_policy_para: *const CertChainPolicyPara,
        p_policy_status: *mut CertChainPolicyStatus,
    ) -> i32;

    fn CertFreeCertificateChain(p_chain_context: *const c_void);
    fn CertFreeCertificateContext(p_cert_context: *const c_void) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLastError() -> u32;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn wide_null(s: &str) -> Vec<u16> {
    let mut wide = Vec::with_capacity(s.len() + 1);
    let mut buf = [0u16; 2];
    for ch in s.chars() {
        let ch = if ch == '\0' { ' ' } else { ch };
        wide.extend_from_slice(ch.encode_utf16(&mut buf));
    }
    wide.push(0);
    wide
}

fn status_text(status: SecStatus) -> &'static str {
    match status {
        SEC_E_UNTRUSTED_ROOT => "untrusted certificate root",
        SEC_E_WRONG_PRINCIPAL => "certificate hostname mismatch",
        SEC_E_CERT_EXPIRED => "expired certificate",
        SEC_E_CERT_UNKNOWN => "unknown certificate error",
        SEC_E_ILLEGAL_MESSAGE => "illegal TLS message",
        SEC_E_INVALID_TOKEN => "invalid TLS token",
        SEC_E_MESSAGE_ALTERED => "TLS message altered in transit",
        SEC_E_OUT_OF_SEQUENCE => "TLS message out of sequence",
        SEC_E_DECRYPT_FAILURE => "TLS decryption failure",
        SEC_E_ENCRYPT_FAILURE => "TLS encryption failure",
        SEC_E_ALGORITHM_MISMATCH => "TLS algorithm mismatch",
        _ => "TLS protocol error",
    }
}

fn error_from_status(context: &str, status: SecStatus) -> NetError {
    NetError {
        message: format!(
            "{context} (Schannel status 0x{:08X}: {})",
            status as u32,
            status_text(status)
        ),
    }
}

fn read_exact_deadline(
    tcp: &mut TcpStream,
    buf: &mut [u8],
    deadline: Instant,
) -> std::io::Result<()> {
    let mut offset = 0usize;
    while offset < buf.len() {
        match tcp.read(&mut buf[offset..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "TLS peer closed the connection during handshake",
                ));
            }
            Ok(n) => offset += n,
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                // One socket wait expired; keep waiting while the total budget lasts (DROP-111).
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(std::io::Error::new(
                        ErrorKind::TimedOut,
                        "timed out reading TLS handshake data",
                    ));
                }
                let _ = tcp.set_read_timeout(Some(remaining.max(Duration::from_millis(1))));
                continue;
            }
            Err(err) => return Err(err),
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                ErrorKind::TimedOut,
                "timed out reading TLS handshake data",
            ));
        }
    }
    Ok(())
}

/// Reads exactly one complete TLS record and appends it to `out`.
fn read_tls_record(
    tcp: &mut TcpStream,
    out: &mut Vec<u8>,
    deadline: Instant,
) -> std::io::Result<()> {
    let mut header = [0u8; 5];
    read_exact_deadline(tcp, &mut header, deadline)?;
    let content_type = header[0];
    if !(20..=23).contains(&content_type) {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("unexpected TLS record content type {content_type}"),
        ));
    }
    let len = u16::from_be_bytes([header[3], header[4]]) as usize;
    if len > MAX_TLS_RECORD {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("TLS record length {len} exceeds maximum"),
        ));
    }
    let start = out.len();
    out.resize(start + 5 + len, 0);
    out[start..start + 5].copy_from_slice(&header);
    read_exact_deadline(tcp, &mut out[start + 5..], deadline)?;
    Ok(())
}

fn send_handshake_token(tcp: &mut TcpStream, token: &[u8]) -> std::io::Result<()> {
    if !token.is_empty() {
        tcp.write_all(token)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// TLS stream
// ---------------------------------------------------------------------------

pub struct TlsStream {
    tcp: TcpStream,
    cred: SecHandle,
    ctx: SecHandle,
    cred_valid: bool,
    ctx_valid: bool,
    sizes: SecPkgContextStreamSizes,
    pending: Vec<u8>,
    plain: Vec<u8>,
    plain_pos: usize,
    closed: bool,
}

impl Drop for TlsStream {
    fn drop(&mut self) {
        self.close();
    }
}

impl TlsStream {
    pub fn connect(tcp: TcpStream, host: &str) -> Result<Self, NetError> {
        let clean_host = strip_ipv6_brackets(host);
        let target = wide_null(clean_host);

        let mut stream = Self {
            tcp,
            cred: SecHandle::default(),
            ctx: SecHandle::default(),
            cred_valid: false,
            ctx_valid: false,
            sizes: SecPkgContextStreamSizes::default(),
            pending: Vec::new(),
            plain: Vec::new(),
            plain_pos: 0,
            closed: false,
        };

        stream.acquire_credentials()?;
        let idle_read = stream.tcp.read_timeout().ok().flatten();
        let leftover = stream.handshake(&target);
        let _ = stream.tcp.set_read_timeout(idle_read);
        let leftover = leftover?;
        stream.pending = leftover;
        stream.query_stream_sizes()?;
        stream.verify_peer(clean_host)?;
        Ok(stream)
    }

    fn acquire_credentials(&mut self) -> Result<(), NetError> {
        let package = wide_null("Microsoft Unified Security Protocol Provider");
        let mut cred = SchannelCred {
            dw_version: SCHANNEL_CRED_VERSION,
            c_creds: 0,
            pa_cred: ptr::null_mut(),
            h_root_store: ptr::null_mut(),
            c_mappers: 0,
            aph_mappers: ptr::null_mut(),
            c_supported_algs: 0,
            palg_supported_algs: ptr::null_mut(),
            grbit_enabled_protocols: 0,
            dw_minimum_cipher_strength: 0,
            dw_maximum_cipher_strength: 0,
            dw_session_lifespan: 0,
            dw_flags: SCH_CRED_AUTO_CRED_VALIDATION
                | SCH_CRED_NO_DEFAULT_CREDS
                | SCH_USE_STRONG_CRYPTO,
            dw_cred_format: 0,
        };
        let mut expiry = TimeStamp::default();
        let status = unsafe {
            AcquireCredentialsHandleW(
                ptr::null(),
                package.as_ptr(),
                SECPKG_CRED_OUTBOUND,
                ptr::null_mut(),
                (&mut cred as *mut SchannelCred).cast::<c_void>(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut self.cred,
                &mut expiry,
            )
        };
        if status != SEC_E_OK {
            return Err(error_from_status(
                "Failed to acquire Schannel client credentials",
                status,
            ));
        }
        self.cred_valid = true;
        Ok(())
    }

    fn handshake(&mut self, target: &[u16]) -> Result<Vec<u8>, NetError> {
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        let mut input = Vec::new();
        let mut first_call = true;

        loop {
            let (status, token, extra) =
                unsafe { self.initialize_security_context(first_call, target, &input)? };
            first_call = false;

            send_handshake_token(&mut self.tcp, &token).map_err(|err| NetError {
                message: format!("Failed to send Schannel handshake token: {err}"),
            })?;

            match status {
                SEC_E_OK => {
                    return Ok(extra);
                }
                SEC_I_CONTINUE_NEEDED => {
                    input = extra;
                    if input.is_empty() {
                        read_tls_record(&mut self.tcp, &mut input, deadline).map_err(|err| {
                            NetError {
                                message: format!("TLS handshake read failed: {err}"),
                            }
                        })?;
                    }
                }
                SEC_E_INCOMPLETE_MESSAGE => {
                    read_tls_record(&mut self.tcp, &mut input, deadline).map_err(|err| {
                        NetError {
                            message: format!("TLS handshake read failed: {err}"),
                        }
                    })?;
                }
                SEC_I_INCOMPLETE_CREDENTIALS => {
                    return Err(NetError {
                        message: "Server requested a TLS client certificate, which Droplet does not support".into(),
                    });
                }
                SEC_I_RENEGOTIATE => {
                    return Err(NetError {
                        message: "Server-initiated TLS renegotiation is not supported".into(),
                    });
                }
                other => {
                    return Err(error_from_status("TLS handshake failed", other));
                }
            }

            if input.len() > MAX_HANDSHAKE_BYTES {
                return Err(NetError {
                    message: "TLS handshake exceeded maximum buffered size".into(),
                });
            }
        }
    }

    /// Calls `InitializeSecurityContextW` and copies both the output token and
    /// any unconsumed input (`SECBUFFER_EXTRA`) into owned buffers.
    unsafe fn initialize_security_context(
        &mut self,
        first_call: bool,
        target: &[u16],
        input: &[u8],
    ) -> Result<(SecStatus, Vec<u8>, Vec<u8>), NetError> {
        let mut new_ctx = SecHandle::default();
        let mut output_buffer = SecBuffer {
            cb_buffer: 0,
            buffer_type: SECBUFFER_TOKEN,
            pv_buffer: ptr::null_mut(),
        };
        let mut output_desc = SecBufferDesc {
            ul_version: SECBUFFER_VERSION,
            c_buffers: 1,
            p_buffers: &mut output_buffer,
        };

        let mut input_buffers = [
            SecBuffer {
                cb_buffer: input.len().min(u32::MAX as usize) as u32,
                buffer_type: SECBUFFER_DATA,
                pv_buffer: input.as_ptr().cast_mut().cast::<c_void>(),
            },
            SecBuffer {
                cb_buffer: 0,
                buffer_type: SECBUFFER_EMPTY,
                pv_buffer: ptr::null_mut(),
            },
        ];
        let mut input_desc = SecBufferDesc {
            ul_version: SECBUFFER_VERSION,
            c_buffers: 2,
            p_buffers: input_buffers.as_mut_ptr(),
        };

        let mut context_attr: u32 = 0;
        let mut expiry = TimeStamp::default();
        let ph_context = if first_call {
            ptr::null_mut()
        } else {
            &mut self.ctx
        };
        let p_input = if input.is_empty() {
            ptr::null_mut()
        } else {
            &mut input_desc
        };

        let flags = ISC_REQ_STREAM
            | ISC_REQ_CONFIDENTIALITY
            | ISC_REQ_SEQUENCE_DETECT
            | ISC_REQ_REPLAY_DETECT
            | ISC_REQ_EXTENDED_ERROR
            | ISC_REQ_ALLOCATE_MEMORY;

        let status = InitializeSecurityContextW(
            &mut self.cred,
            ph_context,
            target.as_ptr(),
            flags,
            0,
            SECURITY_NATIVE_DREP,
            p_input,
            0,
            &mut new_ctx,
            &mut output_desc,
            &mut context_attr,
            &mut expiry,
        );

        let mut token = Vec::new();
        if !output_buffer.pv_buffer.is_null() {
            if output_buffer.cb_buffer > 0 {
                token.extend_from_slice(slice::from_raw_parts(
                    output_buffer.pv_buffer.cast::<u8>(),
                    output_buffer.cb_buffer as usize,
                ));
            }
            FreeContextBuffer(output_buffer.pv_buffer);
        }

        let mut extra = Vec::new();
        for buffer in input_buffers.iter() {
            if buffer.buffer_type == SECBUFFER_EXTRA && !buffer.pv_buffer.is_null() {
                extra.extend_from_slice(slice::from_raw_parts(
                    buffer.pv_buffer.cast::<u8>(),
                    buffer.cb_buffer as usize,
                ));
            }
        }

        let new_ctx_valid = new_ctx.dw_lower != 0 || new_ctx.dw_upper != 0;
        if new_ctx_valid
            && (new_ctx.dw_lower != self.ctx.dw_lower || new_ctx.dw_upper != self.ctx.dw_upper)
        {
            if self.ctx_valid {
                DeleteSecurityContext(&mut self.ctx);
            }
            self.ctx = new_ctx;
            self.ctx_valid = true;
        }

        Ok((status, token, extra))
    }

    fn query_stream_sizes(&mut self) -> Result<(), NetError> {
        let status = unsafe {
            QueryContextAttributesW(
                &mut self.ctx,
                SECPKG_ATTR_STREAM_SIZES,
                (&mut self.sizes as *mut SecPkgContextStreamSizes).cast::<c_void>(),
            )
        };
        if status != SEC_E_OK {
            return Err(error_from_status(
                "Failed to query Schannel stream sizes",
                status,
            ));
        }
        if self.sizes.cb_maximum_message == 0 {
            return Err(NetError {
                message: "Schannel reported a zero maximum TLS message size".into(),
            });
        }
        Ok(())
    }

    fn verify_peer(&mut self, host: &str) -> Result<(), NetError> {
        let mut cert_context: *mut c_void = ptr::null_mut();
        let status = unsafe {
            QueryContextAttributesW(
                &mut self.ctx,
                SECPKG_ATTR_REMOTE_CERT_CONTEXT,
                (&mut cert_context as *mut *mut c_void).cast::<c_void>(),
            )
        };
        if status != SEC_E_OK {
            return Err(error_from_status(
                "Failed to obtain the server certificate",
                status,
            ));
        }
        if cert_context.is_null() {
            return Err(NetError {
                message: "Schannel returned a null server certificate".into(),
            });
        }

        let server_name = wide_null(host);
        let chain_result = unsafe { verify_certificate_chain(cert_context, &server_name) };
        unsafe {
            CertFreeCertificateContext(cert_context);
        }
        chain_result
    }

    fn decrypt_pending(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        if self.pending.is_empty() {
            return Ok(None);
        }
        if self.pending.len() > MAX_PENDING_CIPHERTEXT {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "buffered TLS ciphertext exceeds maximum size",
            ));
        }

        let mut buffers = [
            SecBuffer {
                cb_buffer: 0,
                buffer_type: SECBUFFER_STREAM_HEADER,
                pv_buffer: ptr::null_mut(),
            },
            SecBuffer {
                cb_buffer: self.pending.len() as u32,
                buffer_type: SECBUFFER_DATA,
                pv_buffer: self.pending.as_mut_ptr().cast::<c_void>(),
            },
            SecBuffer {
                cb_buffer: 0,
                buffer_type: SECBUFFER_STREAM_TRAILER,
                pv_buffer: ptr::null_mut(),
            },
            SecBuffer {
                cb_buffer: 0,
                buffer_type: SECBUFFER_EMPTY,
                pv_buffer: ptr::null_mut(),
            },
        ];
        let mut desc = SecBufferDesc {
            ul_version: SECBUFFER_VERSION,
            c_buffers: buffers.len() as u32,
            p_buffers: buffers.as_mut_ptr(),
        };
        let mut qop: u32 = 0;
        let status = unsafe { DecryptMessage(&mut self.ctx, &mut desc, 0, &mut qop) };

        match status {
            SEC_E_OK => {
                let mut plain = Vec::new();
                let mut extra = Vec::new();
                for buffer in buffers.iter() {
                    if buffer.pv_buffer.is_null() {
                        continue;
                    }
                    let bytes = unsafe {
                        slice::from_raw_parts(buffer.pv_buffer.cast::<u8>(), buffer.cb_buffer as usize)
                    };
                    match buffer.buffer_type {
                        SECBUFFER_DATA => plain.extend_from_slice(bytes),
                        SECBUFFER_EXTRA => extra.extend_from_slice(bytes),
                        _ => {}
                    }
                }
                self.pending = extra;
                Ok(Some(plain))
            }
            SEC_E_INCOMPLETE_MESSAGE => Ok(None),
            SEC_I_CONTEXT_EXPIRED => {
                self.pending.clear();
                self.closed = true;
                Ok(Some(Vec::new()))
            }
            SEC_I_RENEGOTIATE => Err(std::io::Error::new(
                ErrorKind::Unsupported,
                "server-initiated TLS renegotiation is not supported",
            )),
            SEC_E_MESSAGE_ALTERED => Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "TLS record was altered in transit",
            )),
            SEC_E_OUT_OF_SEQUENCE => Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "TLS record was received out of sequence",
            )),
            other => Err(std::io::Error::other(format!(
                "Schannel DecryptMessage failed (status 0x{:08X})",
                other as u32
            ))),
        }
    }

    fn fill_pending_from_socket(&mut self) -> std::io::Result<bool> {
        let mut tmp = [0u8; 16 * 1024];
        loop {
            match self.tcp.read(&mut tmp) {
                Ok(0) => return Ok(false),
                Ok(n) => {
                    self.pending.extend_from_slice(&tmp[..n]);
                    return Ok(true);
                }
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    return Err(std::io::Error::new(
                        ErrorKind::TimedOut,
                        "TLS read timed out",
                    ));
                }
                Err(err) => return Err(err),
            }
        }
    }

    pub fn close(&mut self) {
        if self.closed && !self.ctx_valid && !self.cred_valid {
            return;
        }
        self.closed = true;
        if self.ctx_valid {
            unsafe {
                DeleteSecurityContext(&mut self.ctx);
            }
            self.ctx = SecHandle::default();
            self.ctx_valid = false;
        }
        if self.cred_valid {
            unsafe {
                FreeCredentialsHandle(&mut self.cred);
            }
            self.cred = SecHandle::default();
            self.cred_valid = false;
        }
    }
}

impl Read for TlsStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.plain_pos < self.plain.len() {
            let available = self.plain.len() - self.plain_pos;
            let take = available.min(buf.len());
            buf[..take].copy_from_slice(&self.plain[self.plain_pos..self.plain_pos + take]);
            self.plain_pos += take;
            if self.plain_pos >= self.plain.len() {
                self.plain.clear();
                self.plain_pos = 0;
            }
            return Ok(take);
        }
        if self.closed {
            return Ok(0);
        }

        let deadline = Instant::now() + READ_TIMEOUT;
        loop {
            if let Some(plain) = self.decrypt_pending()? {
                if !plain.is_empty() {
                    let take = plain.len().min(buf.len());
                    buf[..take].copy_from_slice(&plain[..take]);
                    if take < plain.len() {
                        self.plain = plain;
                        self.plain_pos = take;
                    }
                    return Ok(take);
                }
                if self.closed {
                    return Ok(0);
                }
                continue;
            }

            if Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    ErrorKind::TimedOut,
                    "TLS read timed out",
                ));
            }
            if !self.fill_pending_from_socket()? {
                if self.pending.is_empty() {
                    self.closed = true;
                    return Ok(0);
                }
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "TLS connection closed with a partial record buffered",
                ));
            }
        }
    }
}

impl Write for TlsStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.closed {
            return Err(std::io::Error::new(
                ErrorKind::BrokenPipe,
                "TLS connection is closed",
            ));
        }

        let max_message = self.sizes.cb_maximum_message as usize;
        let take = buf.len().min(max_message);
        let header_len = self.sizes.cb_header as usize;
        let trailer_len = self.sizes.cb_trailer as usize;
        let mut encrypted = vec![0u8; header_len + take + trailer_len];

        let (header_part, rest) = encrypted.split_at_mut(header_len);
        let (data_part, trailer_part) = rest.split_at_mut(take);
        data_part.copy_from_slice(&buf[..take]);

        let mut buffers = [
            SecBuffer {
                cb_buffer: header_part.len() as u32,
                buffer_type: SECBUFFER_STREAM_HEADER,
                pv_buffer: header_part.as_mut_ptr().cast::<c_void>(),
            },
            SecBuffer {
                cb_buffer: data_part.len() as u32,
                buffer_type: SECBUFFER_DATA,
                pv_buffer: data_part.as_mut_ptr().cast::<c_void>(),
            },
            SecBuffer {
                cb_buffer: trailer_part.len() as u32,
                buffer_type: SECBUFFER_STREAM_TRAILER,
                pv_buffer: trailer_part.as_mut_ptr().cast::<c_void>(),
            },
            SecBuffer {
                cb_buffer: 0,
                buffer_type: SECBUFFER_EMPTY,
                pv_buffer: ptr::null_mut(),
            },
        ];
        let mut desc = SecBufferDesc {
            ul_version: SECBUFFER_VERSION,
            c_buffers: buffers.len() as u32,
            p_buffers: buffers.as_mut_ptr(),
        };

        let status = unsafe { EncryptMessage(&mut self.ctx, 0, &mut desc, 0) };
        if status != SEC_E_OK {
            return Err(std::io::Error::other(format!(
                "Schannel EncryptMessage failed (status 0x{:08X})",
                status as u32
            )));
        }

        let actual_header = buffers[0].cb_buffer as usize;
        let actual_data = buffers[1].cb_buffer as usize;
        let actual_trailer = buffers[2].cb_buffer as usize;
        if actual_header == header_len && actual_data == take {
            self.tcp
                .write_all(&encrypted[..actual_header + actual_data + actual_trailer])?;
        } else {
            let mut packed =
                Vec::with_capacity(actual_header + actual_data + actual_trailer);
            packed.extend_from_slice(&encrypted[..actual_header]);
            packed.extend_from_slice(&encrypted[header_len..header_len + actual_data]);
            packed.extend_from_slice(
                &encrypted[header_len + take..header_len + take + actual_trailer],
            );
            self.tcp.write_all(&packed)?;
        }
        Ok(take)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.tcp.flush()
    }
}

/// Builds the certificate chain and applies the `CERT_CHAIN_POLICY_SSL` policy
/// (system trust, validity window, EKU, and hostname match).
unsafe fn verify_certificate_chain(
    cert_context: *mut c_void,
    server_name: &[u16],
) -> Result<(), NetError> {
    let chain_para = CertChainPara {
        cb_size: mem::size_of::<CertChainPara>() as u32,
        requested_usage: CertUsageMatch {
            dw_type: 0,
            usage: CertEnhKeyUsage {
                c_usage_identifier: 0,
                rgpsz_usage_identifier: ptr::null_mut(),
            },
        },
        requested_issuance_policy: CertUsageMatch {
            dw_type: 0,
            usage: CertEnhKeyUsage {
                c_usage_identifier: 0,
                rgpsz_usage_identifier: ptr::null_mut(),
            },
        },
        dw_url_retrieval_timeout: 0,
        f_check_revocation_freshness_time: 0,
        dw_revocation_freshness_time: 0,
        pft_cache_resync: ptr::null_mut(),
        p_strong_sign_para: ptr::null_mut(),
        dw_strong_sign_flags: 0,
    };

    let mut chain_context: *mut c_void = ptr::null_mut();
    let ok = CertGetCertificateChain(
        ptr::null_mut(),
        cert_context,
        ptr::null_mut(),
        ptr::null_mut(),
        &chain_para,
        0,
        ptr::null_mut(),
        &mut chain_context,
    );
    if ok == 0 || chain_context.is_null() {
        let err = GetLastError();
        return Err(NetError {
            message: format!("Failed to build the server certificate chain (Win32 error {err})"),
        });
    }

    let mut extra = HttpsPolicyCallbackData {
        cb_size: mem::size_of::<HttpsPolicyCallbackData>() as u32,
        dw_auth_type: AUTHTYPE_SERVER,
        fdw_checks: 0,
        pwsz_server_name: server_name.as_ptr(),
    };
    let policy_para = CertChainPolicyPara {
        cb_size: mem::size_of::<CertChainPolicyPara>() as u32,
        dw_flags: 0,
        pv_extra_policy_para: (&mut extra as *mut HttpsPolicyCallbackData).cast::<c_void>(),
    };
    let mut policy_status = CertChainPolicyStatus {
        cb_size: mem::size_of::<CertChainPolicyStatus>() as u32,
        dw_error: 0,
        l_chain_index: 0,
        l_element_index: 0,
        pv_extra_policy_status: ptr::null_mut(),
    };

    let ok = CertVerifyCertificateChainPolicy(
        CERT_CHAIN_POLICY_SSL as *const c_char,
        chain_context,
        &policy_para,
        &mut policy_status,
    );
    let verify_error = if ok == 0 {
        Some(GetLastError())
    } else {
        None
    };
    CertFreeCertificateChain(chain_context);

    if let Some(err) = verify_error {
        return Err(NetError {
            message: format!("TLS certificate policy validation failed (Win32 error {err})"),
        });
    }
    if policy_status.dw_error != 0 {
        return Err(NetError {
            message: format!(
                "TLS certificate verification failed (CryptoAPI error 0x{:08X})",
                policy_status.dw_error
            ),
        });
    }
    Ok(())
}
