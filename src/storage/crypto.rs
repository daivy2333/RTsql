//! 页级加密内核：Argon2id KDF + 页级 AES-256-GCM transform。
//!
//! 加密库每页磁盘记录布局：`12B nonce || 4096B ciphertext || 16B GCM tag`，
//! 记录总长 [`ENCRYPTED_PAGE_RECORD_SIZE`] = 4124B（明文库 4096B 不变）。
//! AAD 绑定 `page_id`（u64 LE），密文页整体搬到其他页位置将无法通过认证。
//! nonce 每次写入经 OS 随机源新生成；KDF 参数三元组持久化于文件头保留区，
//! 打开时按头参数派生，非法值域在头 decode 期由 [`validate_kdf_params`] 拒绝。

use aes_gcm::aead::{Aead, Nonce, Payload};
use aes_gcm::{Aes256Gcm, Key, KeyInit};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;

use super::page_id::PageId;

/// 加密库单页磁盘记录长度：nonce(12) + ciphertext(4096) + tag(16)
pub(crate) const ENCRYPTED_PAGE_RECORD_SIZE: usize = 4124;
/// GCM nonce 长度
pub(crate) const NONCE_SIZE: usize = 12;
/// AES-256 密钥长度
pub(crate) const KEY_SIZE: usize = 32;
/// 默认 KDF 参数：m=19456 KiB、t=2、p=1（OWASP 2023 首选档，
/// 随新建加密库持久化于文件头参数区）
pub(crate) const DEFAULT_KDF_M_KIB: u32 = 19456;
pub(crate) const DEFAULT_KDF_T: u32 = 2;
pub(crate) const DEFAULT_KDF_P: u32 = 1;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CryptoError {
    /// GCM 认证失败（错误密钥、密文/AAD 被篡改）
    AuthFailed,
    /// KDF 参数值域违规
    InvalidParams,
}

/// 校验 KDF 参数值域：`t >= 1`、`p >= 1`、`m_kib ∈ 1024..=2^24`。
pub(crate) fn validate_kdf_params(m_kib: u32, t: u32, p: u32) -> Result<(), CryptoError> {
    if t == 0 || p == 0 || !(1024..=(1 << 24)).contains(&m_kib) {
        return Err(CryptoError::InvalidParams);
    }
    Ok(())
}

/// 从密码派生 [`KEY_SIZE`] 字节 AES-256 密钥（Argon2id）。
///
/// 调用前提：参数已通过 [`validate_kdf_params`] 校验（文件头 decode 是唯一参数来源），
/// 因此本函数对合法参数不存在失败路径。
pub(crate) fn derive_key(password: &[u8], salt: &[u8; 32], m_kib: u32, t: u32, p: u32) -> [u8; 32] {
    let params = Params::new(m_kib, t, p, Some(KEY_SIZE))
        .expect("kdf params validated by validate_kdf_params");
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; KEY_SIZE];
    argon
        .hash_password_into(password, salt, &mut out)
        .expect("argon2 hash into 32B buffer cannot fail for validated params");
    out
}

/// 持有派生密钥的页加密器，封装单页明/密 transform。
pub(crate) struct PageCipher {
    cipher: Aes256Gcm,
}

impl PageCipher {
    pub(crate) fn new(key: [u8; KEY_SIZE]) -> Self {
        let key = Key::<Aes256Gcm>::try_from(key.as_slice())
            .expect("key length is fixed at compile time");
        Self {
            cipher: Aes256Gcm::new(&key),
        }
    }

    /// 加密单页：随机 nonce + AES-256-GCM，输出 `nonce || ciphertext || tag`。
    /// 固定 4KiB 输入 + 合法密钥下 GCM 加密无失败路径。
    pub(crate) fn encrypt_page(
        &self,
        page_id: PageId,
        page: &[u8; 4096],
    ) -> [u8; ENCRYPTED_PAGE_RECORD_SIZE] {
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::<Aes256Gcm>::try_from(nonce_bytes.as_slice())
            .expect("nonce length is fixed at compile time");
        let ciphertext = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: page.as_slice(),
                    aad: &page_id.0.to_le_bytes(),
                },
            )
            .expect("AES-GCM encryption of a fixed 4KiB page cannot fail");
        debug_assert_eq!(ciphertext.len(), 4096 + 16);
        let mut record = [0u8; ENCRYPTED_PAGE_RECORD_SIZE];
        record[..NONCE_SIZE].copy_from_slice(&nonce_bytes);
        record[NONCE_SIZE..].copy_from_slice(&ciphertext);
        record
    }

    /// 解密单页记录：认证失败（错误密钥 / 密文或 AAD 篡改）返回 [`CryptoError::AuthFailed`]。
    pub(crate) fn decrypt_page(
        &self,
        page_id: PageId,
        record: &[u8; ENCRYPTED_PAGE_RECORD_SIZE],
    ) -> Result<[u8; 4096], CryptoError> {
        let nonce = Nonce::<Aes256Gcm>::try_from(&record[..NONCE_SIZE])
            .expect("nonce length is fixed at compile time");
        let plaintext = self
            .cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &record[NONCE_SIZE..],
                    aad: &page_id.0.to_le_bytes(),
                },
            )
            .map_err(|_| CryptoError::AuthFailed)?;
        let mut page = [0u8; 4096];
        page.copy_from_slice(&plaintext);
        Ok(page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M_KIB: u32 = 19456;
    const T: u32 = 2;
    const P: u32 = 1;

    fn test_cipher() -> PageCipher {
        PageCipher::new(derive_key(b"password", &[7u8; 32], M_KIB, T, P))
    }

    #[test]
    fn param_range_rejected() {
        assert_eq!(validate_kdf_params(M_KIB, T, P), Ok(()));
        assert_eq!(validate_kdf_params(1024, 1, 1), Ok(()));
        assert_eq!(validate_kdf_params(1 << 24, 1, 1), Ok(()));
        assert_eq!(
            validate_kdf_params(M_KIB, 0, P),
            Err(CryptoError::InvalidParams)
        );
        assert_eq!(
            validate_kdf_params(M_KIB, T, 0),
            Err(CryptoError::InvalidParams)
        );
        assert_eq!(
            validate_kdf_params(512, T, P),
            Err(CryptoError::InvalidParams)
        );
        assert_eq!(
            validate_kdf_params((1 << 24) + 1, T, P),
            Err(CryptoError::InvalidParams)
        );
    }

    #[test]
    fn kdf_deterministic_and_salt_sensitive() {
        let salt_a = [1u8; 32];
        let salt_b = [2u8; 32];
        let k1 = derive_key(b"password", &salt_a, M_KIB, T, P);
        let k2 = derive_key(b"password", &salt_a, M_KIB, T, P);
        let k3 = derive_key(b"password", &salt_b, M_KIB, T, P);
        let k4 = derive_key(b"passphrase", &salt_a, M_KIB, T, P);
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
        assert_ne!(k1, k4);
    }

    #[test]
    fn page_roundtrip() {
        let cipher = test_cipher();
        let page = [0xA5u8; 4096];
        let record = cipher.encrypt_page(PageId(3), &page);
        assert_eq!(record.len(), ENCRYPTED_PAGE_RECORD_SIZE);
        assert_eq!(cipher.decrypt_page(PageId(3), &record), Ok(page));
    }

    #[test]
    fn wrong_key_auth_failed() {
        let page = [0x5Au8; 4096];
        let record = test_cipher().encrypt_page(PageId(0), &page);
        let other = PageCipher::new(derive_key(b"wrong", &[7u8; 32], M_KIB, T, P));
        assert_eq!(
            other.decrypt_page(PageId(0), &record),
            Err(CryptoError::AuthFailed)
        );
    }

    #[test]
    fn aad_page_id_swap_rejected() {
        let page = [0x11u8; 4096];
        let record = test_cipher().encrypt_page(PageId(1), &page);
        assert_eq!(
            test_cipher().decrypt_page(PageId(2), &record),
            Err(CryptoError::AuthFailed)
        );
    }

    #[test]
    fn ciphertext_bitflip_rejected() {
        let page = [0x22u8; 4096];
        let mut record = test_cipher().encrypt_page(PageId(4), &page);
        record[NONCE_SIZE] ^= 0x01;
        assert_eq!(
            test_cipher().decrypt_page(PageId(4), &record),
            Err(CryptoError::AuthFailed)
        );
    }

    #[test]
    fn tag_bitflip_rejected() {
        let page = [0x33u8; 4096];
        let mut record = test_cipher().encrypt_page(PageId(4), &page);
        let last = ENCRYPTED_PAGE_RECORD_SIZE - 1;
        record[last] ^= 0x80;
        assert_eq!(
            test_cipher().decrypt_page(PageId(4), &record),
            Err(CryptoError::AuthFailed)
        );
    }

    #[test]
    fn nonce_unique_per_write() {
        let cipher = test_cipher();
        let page = [0x44u8; 4096];
        let r1 = cipher.encrypt_page(PageId(5), &page);
        let r2 = cipher.encrypt_page(PageId(5), &page);
        assert_ne!(&r1[..NONCE_SIZE], &r2[..NONCE_SIZE]);
        // 不同 nonce 下解密均还原同一明文
        assert_eq!(cipher.decrypt_page(PageId(5), &r2), Ok(page));
    }
}
