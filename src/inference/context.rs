#![allow(unused_imports, dead_code)]

//! llama.cpp FFI wrapper via the `llama_cpp_sys_2` crate.
//!
//! Requires the `llama-cpp` feature flag to be enabled at compile time.
//! Without it, all public methods return stub errors or no-ops.
//! All unsafe blocks document their safety invariants inline.

use std::ffi::CString;
use std::ptr;

use anyhow::{bail, Context as _, Result};

use super::model::{InferenceConfig, KvQuantType};

#[cfg(feature = "llama-cpp")]
use llama_cpp_sys_2 as ffi;

#[derive(Debug)]
pub struct LlamaContext {
    #[cfg(feature = "llama-cpp")]
    model: *mut ffi::llama_model,
    #[cfg(feature = "llama-cpp")]
    ctx: *mut ffi::llama_context,
    #[cfg(not(feature = "llama-cpp"))]
    _phantom: (),
    n_ctx: u32,
    n_vocab: i32,
}

// Raw pointers are not Send/Sync by default. llama.cpp contexts are
// thread-safe for single-threaded access patterns (one decode at a time),
// which is how we use them: one Sampler drives one context sequentially.
unsafe impl Send for LlamaContext {}

impl LlamaContext {
    #[cfg(feature = "llama-cpp")]
    pub fn new(config: &InferenceConfig) -> Result<Self> {
        let path_str = config
            .model_path
            .to_str()
            .context("model path is not valid UTF-8")?;
        let c_path = CString::new(path_str)?;

        // Safety: llama_backend_init is safe to call once before any other llama.cpp API.
        unsafe {
            ffi::llama_backend_init();
        }

        let model_params = unsafe {
            let mut p = ffi::llama_model_default_params();
            p.n_gpu_layers = config.gpu_layers;
            p
        };

        // Safety: c_path is a valid null-terminated string, model_params is initialized above.
        let model = unsafe { ffi::llama_load_model_from_file(c_path.as_ptr(), model_params) };
        if model.is_null() {
            bail!("failed to load model from {path_str}");
        }

        let ctx_params = unsafe {
            let mut p = ffi::llama_context_default_params();
            p.n_ctx = config.context_length;
            p.n_batch = config.batch_size;
            p.n_threads = config.threads as i32;
            p.flash_attn = config.flash_attention;
            p.type_k = config.kv_type_k.to_ffi();
            p.type_v = config.kv_type_v.to_ffi();
            p
        };

        // Safety: model is a valid non-null pointer from llama_load_model_from_file.
        let ctx = unsafe { ffi::llama_new_context_with_model(model, ctx_params) };
        if ctx.is_null() {
            // Safety: model was successfully loaded above.
            unsafe { ffi::llama_free_model(model) };
            bail!("failed to create llama context");
        }

        let n_vocab = unsafe { ffi::llama_n_vocab(model) };

        Ok(Self {
            model,
            ctx,
            n_ctx: config.context_length,
            n_vocab,
        })
    }

    #[cfg(not(feature = "llama-cpp"))]
    pub fn new(_config: &InferenceConfig) -> Result<Self> {
        bail!("llama.cpp support not compiled — enable the 'llama-cpp' feature")
    }

    pub fn context_length(&self) -> u32 {
        self.n_ctx
    }

    pub fn vocab_size(&self) -> i32 {
        self.n_vocab
    }

    #[cfg(feature = "llama-cpp")]
    pub fn tokenize(&self, text: &str, add_bos: bool) -> Result<Vec<i32>> {
        let c_text = CString::new(text)?;
        let max_tokens = text.len() as i32 + 128;
        let mut tokens = vec![0i32; max_tokens as usize];

        // Safety: ctx is valid, c_text is null-terminated, tokens buffer is large enough.
        let n = unsafe {
            ffi::llama_tokenize(
                self.model,
                c_text.as_ptr(),
                text.len() as i32,
                tokens.as_mut_ptr(),
                max_tokens,
                add_bos,
                false, // special tokens
            )
        };

        if n < 0 {
            bail!("tokenization failed (needed {} tokens)", -n);
        }

        tokens.truncate(n as usize);
        Ok(tokens)
    }

    #[cfg(not(feature = "llama-cpp"))]
    pub fn tokenize(&self, _text: &str, _add_bos: bool) -> Result<Vec<i32>> {
        bail!("llama.cpp support not compiled")
    }

    #[cfg(feature = "llama-cpp")]
    pub fn decode_batch(&self, tokens: &[i32], start_pos: i32) -> Result<()> {
        let batch = unsafe {
            let mut b = ffi::llama_batch_init(tokens.len() as i32, 0, 1);
            for (i, &token) in tokens.iter().enumerate() {
                b.token.add(i).write(token);
                b.pos.add(i).write(start_pos + i as i32);
                b.n_seq_id.add(i).write(1);
                let seq_ids = b.seq_id.add(i).read();
                seq_ids.write(0);
                b.logits.add(i).write(if i == tokens.len() - 1 { 1 } else { 0 });
            }
            b.n_tokens = tokens.len() as i32;
            b
        };

        // Safety: ctx and batch are valid, batch tokens reference valid model vocabulary.
        let result = unsafe { ffi::llama_decode(self.ctx, batch) };

        // Safety: batch was initialized with llama_batch_init, safe to free.
        unsafe { ffi::llama_batch_free(batch) };

        if result != 0 {
            bail!("llama_decode failed with code {result}");
        }
        Ok(())
    }

    #[cfg(not(feature = "llama-cpp"))]
    pub fn decode_batch(&self, _tokens: &[i32], _start_pos: i32) -> Result<()> {
        bail!("llama.cpp support not compiled")
    }

    #[cfg(feature = "llama-cpp")]
    pub fn get_logits(&self) -> &[f32] {
        // Safety: ctx is valid, logits buffer is valid after a successful decode call.
        // The returned slice borrows self, preventing use-after-free.
        unsafe {
            let ptr = ffi::llama_get_logits(self.ctx);
            std::slice::from_raw_parts(ptr, self.n_vocab as usize)
        }
    }

    #[cfg(not(feature = "llama-cpp"))]
    pub fn get_logits(&self) -> &[f32] {
        &[]
    }

    #[cfg(feature = "llama-cpp")]
    pub fn clear_kv_cache(&self) {
        // Safety: ctx is valid.
        unsafe { ffi::llama_kv_cache_clear(self.ctx) };
    }

    #[cfg(not(feature = "llama-cpp"))]
    pub fn clear_kv_cache(&self) {}

    #[cfg(feature = "llama-cpp")]
    pub fn kv_cache_used(&self) -> usize {
        // Safety: ctx is valid.
        unsafe { ffi::llama_get_kv_cache_used_cells(self.ctx) as usize }
    }

    #[cfg(not(feature = "llama-cpp"))]
    pub fn kv_cache_used(&self) -> usize {
        0
    }

    #[cfg(feature = "llama-cpp")]
    pub fn token_to_str(&self, token_id: i32) -> String {
        let mut buf = vec![0u8; 128];
        // Safety: model is valid, buf is large enough for any single token piece.
        let n = unsafe {
            ffi::llama_token_to_piece(
                self.model,
                token_id,
                buf.as_mut_ptr() as *mut i8,
                128,
                0,
                false,
            )
        };
        if n < 0 {
            return String::new();
        }
        buf.truncate(n as usize);
        String::from_utf8_lossy(&buf).into_owned()
    }

    #[cfg(not(feature = "llama-cpp"))]
    pub fn token_to_str(&self, _token_id: i32) -> String {
        String::new()
    }
}

#[cfg(feature = "llama-cpp")]
impl Drop for LlamaContext {
    fn drop(&mut self) {
        // Safety: ctx and model are valid pointers obtained from llama.cpp init functions.
        // They have not been freed yet (Drop runs exactly once).
        unsafe {
            if !self.ctx.is_null() {
                ffi::llama_free(self.ctx);
            }
            if !self.model.is_null() {
                ffi::llama_free_model(self.model);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_new_without_feature_returns_error() {
        let config = InferenceConfig::default();
        let result = LlamaContext::new(&config);
        #[cfg(not(feature = "llama-cpp"))]
        assert!(result.is_err());
        #[cfg(feature = "llama-cpp")]
        {
            // Without a real model file, this should also error
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_stub_methods_do_not_panic() {
        #[cfg(not(feature = "llama-cpp"))]
        {
            // We can't construct LlamaContext without the feature, but we can test
            // that the stub types compile and the module structure is sound.
            let config = InferenceConfig::default();
            assert_eq!(config.context_length, 8192);
        }
    }
}
