use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::XChaCha20Poly1305;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Wire format for mesh messages between Forge and Myelin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshMessage {
    pub id: String,
    pub from: String,
    pub message_type: String, // "query", "observe", "sync_request"
    pub payload: serde_json::Value,
    pub timestamp: u64,
    pub sequence: u64,
}

/// Lightweight Myelin mesh client.
///
/// Communicates via filesystem queues:
///   inbox/          — messages TO Myelin
///   outbox/<tool-id>/ — responses FROM Myelin
///   identities/     — public key registry
pub struct MyelinClient {
    mesh_dir: PathBuf,
    tool_id: String,
    signing_key: SigningKey,
    encryption_key: [u8; 32],
    sequence: u64,
}

const NONCE_LEN: usize = 24;
const SIGNATURE_LEN: usize = 64;

impl MyelinClient {
    /// Create or load a Myelin mesh client.
    ///
    /// * `identity_dir` — directory for persisting Forge's Ed25519 keypair
    /// * `mesh_dir` — Myelin's shared mesh directory (e.g. `~/myelin-data/mesh/`)
    /// * `encryption_key` — 32-byte symmetric key shared with Myelin
    pub fn new(identity_dir: &Path, mesh_dir: &Path, encryption_key: [u8; 32]) -> Result<Self> {
        fs::create_dir_all(identity_dir)
            .with_context(|| format!("create identity dir: {}", identity_dir.display()))?;

        let key_path = identity_dir.join("forge.key");
        let signing_key = if key_path.exists() {
            let bytes = fs::read(&key_path)
                .with_context(|| format!("read signing key: {}", key_path.display()))?;
            if bytes.len() != 32 {
                anyhow::bail!(
                    "corrupt signing key at {} (expected 32 bytes, got {})",
                    key_path.display(),
                    bytes.len()
                );
            }
            let mut secret = [0u8; 32];
            secret.copy_from_slice(&bytes);
            SigningKey::from_bytes(&secret)
        } else {
            let key = SigningKey::generate(&mut OsRng);
            fs::write(&key_path, key.to_bytes())
                .with_context(|| format!("write signing key: {}", key_path.display()))?;
            key
        };

        let tool_id = derive_tool_id(&signing_key.verifying_key());

        // Ensure mesh directories exist
        fs::create_dir_all(mesh_dir.join("inbox"))?;
        fs::create_dir_all(mesh_dir.join("outbox").join(&tool_id))?;
        fs::create_dir_all(mesh_dir.join("identities"))?;

        Ok(Self {
            mesh_dir: mesh_dir.to_path_buf(),
            tool_id,
            signing_key,
            encryption_key,
            sequence: 0,
        })
    }

    /// Query Myelin's brain. Sends a query message and polls for a response.
    pub fn query(&mut self, question: &str, timeout_secs: u64) -> Result<Option<String>> {
        let msg_id = uuid::Uuid::new_v4().to_string();
        let msg = MeshMessage {
            id: msg_id.clone(),
            from: self.tool_id.clone(),
            message_type: "query".into(),
            payload: serde_json::json!({ "question": question }),
            timestamp: now_epoch(),
            sequence: self.next_seq(),
        };
        self.send_message(msg)?;
        self.wait_response(&msg_id, timeout_secs)
    }

    /// Submit an observation to Myelin for learning.
    pub fn observe(&mut self, observation: &str, source: &str) -> Result<()> {
        let msg = MeshMessage {
            id: uuid::Uuid::new_v4().to_string(),
            from: self.tool_id.clone(),
            message_type: "observe".into(),
            payload: serde_json::json!({ "observation": observation, "source": source }),
            timestamp: now_epoch(),
            sequence: self.next_seq(),
        };
        self.send_message(msg)
    }

    /// Register this Forge instance with Myelin's tool registry.
    pub fn register(&self) -> Result<()> {
        let pub_path = self
            .mesh_dir
            .join("identities")
            .join(format!("{}.pub", self.tool_id));
        let pubkey_bytes = self.signing_key.verifying_key().to_bytes();
        fs::write(&pub_path, pubkey_bytes)
            .with_context(|| format!("write pubkey to {}", pub_path.display()))?;
        Ok(())
    }

    /// This client's tool ID (first 16 hex chars of SHA-256 of public key).
    pub fn tool_id(&self) -> &str {
        &self.tool_id
    }

    /// Seal data: sign with Ed25519, encrypt with XChaCha20-Poly1305, prepend nonce.
    pub fn seal(&self, data: &[u8]) -> Result<Vec<u8>> {
        // Sign the plaintext
        let signature = self.signing_key.sign(data);
        let sig_bytes = signature.to_bytes();

        // Concatenate signature + plaintext as the payload to encrypt
        let mut plaintext = Vec::with_capacity(SIGNATURE_LEN + data.len());
        plaintext.extend_from_slice(&sig_bytes);
        plaintext.extend_from_slice(data);

        // Encrypt
        let cipher = XChaCha20Poly1305::new_from_slice(&self.encryption_key)
            .map_err(|e| anyhow::anyhow!("cipher init: {e}"))?;
        let nonce = chacha20poly1305::XNonce::from(rand_nonce());
        let ciphertext = cipher
            .encrypt(&nonce, plaintext.as_slice())
            .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;

        let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Open sealed data: decrypt, return plaintext (after stripping signature prefix).
    pub fn open(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < NONCE_LEN {
            anyhow::bail!("sealed data too short (need at least {NONCE_LEN} bytes for nonce)");
        }
        let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
        let nonce = chacha20poly1305::XNonce::from_slice(nonce_bytes);

        let cipher = XChaCha20Poly1305::new_from_slice(&self.encryption_key)
            .map_err(|e| anyhow::anyhow!("cipher init: {e}"))?;
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("decrypt: {e}"))?;

        if plaintext.len() < SIGNATURE_LEN {
            anyhow::bail!("decrypted payload too short for signature");
        }

        // Strip the signature prefix, return the original data
        Ok(plaintext[SIGNATURE_LEN..].to_vec())
    }

    // ── internal ──

    fn send_message(&mut self, msg: MeshMessage) -> Result<()> {
        let json = serde_json::to_vec(&msg)?;
        let sealed = self.seal(&json)?;
        let filename = format!("{}.msg", msg.id);
        let path = self.mesh_dir.join("inbox").join(&filename);
        fs::write(&path, &sealed)
            .with_context(|| format!("write message to {}", path.display()))?;
        Ok(())
    }

    fn wait_response(&self, message_id: &str, timeout_secs: u64) -> Result<Option<String>> {
        let path = self
            .mesh_dir
            .join("outbox")
            .join(&self.tool_id)
            .join(format!("{message_id}.msg"));
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
        while std::time::Instant::now() < deadline {
            if path.exists() {
                let data = fs::read(&path)?;
                let plaintext = self.open(&data)?;
                let text = String::from_utf8(plaintext)
                    .context("response was not valid UTF-8")?;
                // Clean up after reading
                let _ = fs::remove_file(&path);
                return Ok(Some(text));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(None)
    }

    fn next_seq(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }
}

/// Derive tool_id: first 16 hex chars of SHA-256(pubkey).
fn derive_tool_id(pubkey: &VerifyingKey) -> String {
    let hash = Sha256::digest(pubkey.as_bytes());
    hex::encode(&hash[..8]) // 8 bytes = 16 hex chars
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn rand_nonce() -> [u8; NONCE_LEN] {
    use chacha20poly1305::aead::OsRng;
    use rand_core::RngCore;
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    nonce
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_key() -> [u8; 32] {
        [0x42u8; 32]
    }

    fn make_client(identity_dir: &Path, mesh_dir: &Path) -> MyelinClient {
        MyelinClient::new(identity_dir, mesh_dir, test_key()).unwrap()
    }

    #[test]
    fn test_client_generates_identity() {
        let tmp = TempDir::new().unwrap();
        let id_dir = tmp.path().join("identity");
        let mesh_dir = tmp.path().join("mesh");

        let _client = make_client(&id_dir, &mesh_dir);

        // Keypair file should exist
        assert!(id_dir.join("forge.key").exists());
        let key_bytes = fs::read(id_dir.join("forge.key")).unwrap();
        assert_eq!(key_bytes.len(), 32);
    }

    #[test]
    fn test_client_tool_id_deterministic() {
        let tmp = TempDir::new().unwrap();
        let id_dir = tmp.path().join("identity");
        let mesh_dir = tmp.path().join("mesh");

        let client1 = make_client(&id_dir, &mesh_dir);
        let client2 = make_client(&id_dir, &mesh_dir);

        assert_eq!(client1.tool_id(), client2.tool_id());
        assert_eq!(client1.tool_id().len(), 16);
        // Should be valid hex
        assert!(client1.tool_id().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_seal_open_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let client = make_client(&tmp.path().join("id"), &tmp.path().join("mesh"));

        let original = b"hello from forge to myelin";
        let sealed = client.seal(original).unwrap();
        let opened = client.open(&sealed).unwrap();

        assert_eq!(opened, original);
    }

    #[test]
    fn test_send_message_creates_file() {
        let tmp = TempDir::new().unwrap();
        let mesh_dir = tmp.path().join("mesh");
        let mut client = make_client(&tmp.path().join("id"), &mesh_dir);

        client
            .observe("user prefers dark theme", "forge-session")
            .unwrap();

        // Should have exactly one file in inbox
        let inbox = mesh_dir.join("inbox");
        let files: Vec<_> = fs::read_dir(&inbox)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(files.len(), 1);
        assert!(files[0].path().extension().unwrap() == "msg");

        // File should be non-empty sealed data
        let data = fs::read(files[0].path()).unwrap();
        assert!(data.len() > NONCE_LEN + SIGNATURE_LEN);
    }

    #[test]
    fn test_register_writes_pubkey() {
        let tmp = TempDir::new().unwrap();
        let mesh_dir = tmp.path().join("mesh");
        let client = make_client(&tmp.path().join("id"), &mesh_dir);

        client.register().unwrap();

        let pub_path = mesh_dir
            .join("identities")
            .join(format!("{}.pub", client.tool_id()));
        assert!(pub_path.exists());
        let bytes = fs::read(&pub_path).unwrap();
        assert_eq!(bytes.len(), 32); // Ed25519 public key is 32 bytes
    }

    #[test]
    fn test_open_rejects_short_data() {
        let tmp = TempDir::new().unwrap();
        let client = make_client(&tmp.path().join("id"), &tmp.path().join("mesh"));

        let result = client.open(&[0u8; 10]);
        assert!(result.is_err());
    }

    #[test]
    fn test_open_rejects_corrupted_data() {
        let tmp = TempDir::new().unwrap();
        let client = make_client(&tmp.path().join("id"), &tmp.path().join("mesh"));

        let sealed = client.seal(b"test data").unwrap();
        let mut corrupted = sealed.clone();
        // Corrupt a byte in the ciphertext (after nonce)
        if corrupted.len() > NONCE_LEN + 1 {
            corrupted[NONCE_LEN + 1] ^= 0xFF;
        }
        let result = client.open(&corrupted);
        assert!(result.is_err());
    }

    #[test]
    fn test_query_timeout_returns_none() {
        let tmp = TempDir::new().unwrap();
        let mesh_dir = tmp.path().join("mesh");
        let mut client = make_client(&tmp.path().join("id"), &mesh_dir);

        // Query with very short timeout — no response will appear
        let result = client.query("what is forge?", 0).unwrap();
        assert!(result.is_none());
    }
}
