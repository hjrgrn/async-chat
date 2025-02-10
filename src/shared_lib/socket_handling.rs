use core::str;
use std::fmt::Debug;
use std::usize;

use aes_gcm::{
    aead::{
        consts::{B0, B1},
        Aead,
    },
    aes::{
        cipher::typenum::{UInt, UTerm},
        Aes256,
    },
    AeadCore, Aes256Gcm, AesGcm, KeyInit,
};
use anyhow::Context;
use hmac::{
    digest::{generic_array::GenericArray, InvalidLength},
    Hmac, Mac,
};
use rand::rngs::OsRng;
use sha2::Sha256;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::auxiliaries::error_chain_fmt;

/// TODO: comments
pub const PLAIN_PACKET_SIZE: usize = 1024;
pub const PPACKET_PREFIX_SIZE: usize = 4;
pub const PLAIN_PAYLOAD_SIZE: usize = 1020;
pub const CIPTEXT_SIZE: usize = 1040;
pub const CIPTEXT_W_NONCE_SIZE: usize = 1052;
pub const CIPTEXT_W_NONCE_SIZE_AND_HASH: usize = 1084;
pub const NONCE_SIZE: usize = 12;
pub const HMAC_KEY_SIZE: usize = 32;
pub const SEQ_NUM_SIZE: usize = 10;

/// # `RecvHandler`
///
/// This struct handles the receiving side of a socket
/// It takes care also of encryption and message integrity.
/// It acts differently based on the presence of a `cipher`
/// in its fields.
pub struct RecvHandler<T: AsyncRead + Unpin + Send> {
    cursor: usize,
    buffer: [u8; CIPTEXT_W_NONCE_SIZE_AND_HASH],
    plaintext_size: usize,
    pbuffer_prefix_size: usize,
    ciphertext_w_nonce_size: usize,
    ciphertext_w_nonce_size_and_hmac: usize,
    ciphertext_size: usize,
    reader: T,
    s_tools: Option<SafetyTools>,
    seq_num_size: usize,
}

/// FIX: code duplication
impl<T: AsyncRead + Unpin + Send> RecvHandler<T> {
    pub fn new(reader: T) -> Self {
        Self {
            cursor: 0,
            buffer: [0; CIPTEXT_W_NONCE_SIZE_AND_HASH],
            plaintext_size: PLAIN_PACKET_SIZE,
            pbuffer_prefix_size: PPACKET_PREFIX_SIZE,
            ciphertext_w_nonce_size: CIPTEXT_W_NONCE_SIZE,
            ciphertext_w_nonce_size_and_hmac: CIPTEXT_W_NONCE_SIZE_AND_HASH,
            ciphertext_size: CIPTEXT_SIZE,
            reader,
            s_tools: None,
            seq_num_size: SEQ_NUM_SIZE,
        }
    }

    /// # `recv_str`
    ///
    /// Receives a string from the socket being handled.
    /// If a cipher has been set this method expects to receive a ciphertext,
    /// a plaintext otherwise, those two have different sizes.
    /// The message received is assigned to the reference of String received by
    /// the caller.
    pub async fn recv_str(&mut self, line: &mut String) -> Result<(), RecvHandlerError> {
        line.clear();
        // Decide the size of the packet
        let buffer_size = match &self.s_tools {
            Some(_) => self.ciphertext_w_nonce_size_and_hmac,
            None => self.plaintext_size,
        };

        self.receive_packet(buffer_size).await?;

        self.decipher_packet()?;

        let size = self.validate_packet()?;

        // TODO: we may make it more efficient
        let l = str::from_utf8(&self.buffer[self.cursor..self.cursor + size])
            .context("Line sent contained invalid UTF-8")?;
        *line = l.to_string();
        if size != line.len() {
            return Err(RecvHandlerError::MalformedPacket(anyhow::anyhow!(
                "Size prefix and lenght of the line are different."
            )));
        }

        Ok(())
    }

    // TODO: comment, specify use of cursor
    fn validate_packet(&mut self) -> Result<usize, RecvHandlerError> {
        // Validate packet
        let size: usize = str::from_utf8(&self.buffer[0..self.pbuffer_prefix_size])
            .context("Malformed size prefix")?
            .parse()
            .context("Malformed size prefix")?;
        self.cursor = self.pbuffer_prefix_size;
        match &mut self.s_tools {
            Some(s_tools) => {
                let recv_sec_num: u32 =
                    str::from_utf8(&self.buffer[self.cursor..self.cursor + self.seq_num_size])
                        .context("Malformed seq number")?
                        .parse()
                        .context("Malformed seq number")?;
                if recv_sec_num != s_tools.seq_number {
                    return Err(RecvHandlerError::MalformedPacket(anyhow::anyhow!(
                        "Malformed seq number"
                    )));
                }
                s_tools.seq_number = s_tools.seq_number.wrapping_add(1);
                self.cursor = self.cursor + self.seq_num_size;
            }
            None => {}
        }
        if size > self.plaintext_size - self.cursor {
            return Err(RecvHandlerError::MalformedPacket(anyhow::anyhow!(
                "Size of the packet out of bound"
            )));
        }
        Ok(size)
    }

    /// # `recv_str`'s helper function
    async fn receive_packet(&mut self, buffer_size: usize) -> Result<(), RecvHandlerError> {
        self.cursor = 0;

        // Receive packet
        while self.cursor < buffer_size {
            match self.reader.read(&mut self.buffer[self.cursor..]).await {
                Ok(0) => {
                    return Err(RecvHandlerError::ConnectionInterrupted);
                }
                Ok(x) => {
                    self.cursor = self.cursor + x;
                }
                Err(e) => {
                    return Err(RecvHandlerError::IoError(e));
                }
            }
        }
        Ok(())
    }

    /// # `recv_str`'s helper function
    fn decipher_packet(&mut self) -> Result<(), RecvHandlerError> {
        match &self.s_tools {
            Some(s_tools) => {
                let mut hmac = <Hmac<Sha256> as Mac>::new_from_slice(&s_tools.hmac_key)
                    .map_err(|e| RecvHandlerError::HmacError(e.into()))?;
                hmac.update(&self.buffer[..self.ciphertext_w_nonce_size]);
                hmac.verify_slice(&self.buffer[self.ciphertext_w_nonce_size..])
                    .map_err(|e| RecvHandlerError::HmacError(e.into()))?;

                let nonce = &self.buffer[self.ciphertext_size..self.ciphertext_w_nonce_size];

                let buffer = &s_tools
                    .aes_cipher
                    .decrypt(nonce.into(), &self.buffer[..self.ciphertext_size])
                    .map_err(|e| RecvHandlerError::EncryptionError(anyhow::anyhow!(e)))?;

                for i in 0..self.plaintext_size {
                    self.buffer[i] = buffer[i];
                }
            }
            None => {}
        }

        Ok(())
    }

    // `recv_bytes`
    //
    // Same as `recv_str` but it returns a vector of bytes on success.
    // NOTE: For now the encryption is not consider here becouse this method
    // is used only once during the handshake, when the cipher is not
    // defined yet.
    pub async fn recv_bytes(&mut self) -> Result<Vec<u8>, RecvHandlerError> {
        self.receive_packet(self.plaintext_size).await?;
        let size = self.validate_packet()?;

        // TODO: rewrite this without the memcopy
        let vec =
            Vec::from(&self.buffer[self.pbuffer_prefix_size..self.pbuffer_prefix_size + size]);
        if size != vec.len() {
            return Err(RecvHandlerError::MalformedPacket(anyhow::anyhow!(
                "Size prefix and lenght of the line are different."
            )));
        }

        Ok(vec)
    }

    pub fn import_safety_tools(
        &mut self,
        aes_key: &GenericArray<
            u8,
            UInt<UInt<UInt<UInt<UInt<UInt<UTerm, B1>, B0>, B0>, B0>, B0>, B0>,
        >,
        hmac_key_bytes: &[u8],
        seq_number: u32,
    ) -> Result<(), anyhow::Error> {
        if hmac_key_bytes.len() != 32 {
            // This should not happen
            return Err(anyhow::anyhow!("Hmac Key bytes has the wrong size."));
        }
        let mut hmac_key = [0; 32];
        hmac_key.copy_from_slice(hmac_key_bytes);

        self.s_tools = Some(SafetyTools {
            aes_cipher: Aes256Gcm::new(aes_key),
            hmac_key,
            seq_number,
        });
        Ok(())
    }
}

#[derive(thiserror::Error)]
pub enum RecvHandlerError {
    #[error("Peer closed the connection.")]
    ConnectionInterrupted,
    #[error(transparent)]
    IoError(#[from] io::Error),
    #[error(transparent)]
    MalformedPacket(#[from] anyhow::Error),
    // TODO: something better
    #[error(transparent)]
    EncryptionError(anyhow::Error),
    #[error(transparent)]
    HmacError(anyhow::Error),
}

impl Debug for RecvHandlerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        error_chain_fmt(self, f)
    }
}

/// # `WriteHandler`
///
/// This struct handles the writing side of a socket.
/// It takes care also of encryption and message integrity.
/// It acts differently based on the presence of a `cipher`
/// in its fields.
pub struct WriteHandler<T: AsyncWrite + Unpin + Send> {
    cursor: usize,
    buffer: [u8; PLAIN_PACKET_SIZE],
    encrypte_buffer: [u8; CIPTEXT_W_NONCE_SIZE_AND_HASH],
    plaintext_size: usize,
    pbuffer_prefix_size: usize,
    ciphertext_w_nonce_size: usize,
    ciphertext_w_nonce_size_and_hmac: usize,
    writer: T,
    s_tools: Option<SafetyTools>,
    seq_num_size: usize,
    rng: OsRng,
}

impl<T: AsyncWrite + Unpin + Send> WriteHandler<T> {
    pub fn new(writer: T) -> Self {
        Self {
            cursor: 0,
            buffer: [0; PLAIN_PACKET_SIZE],
            encrypte_buffer: [0; CIPTEXT_W_NONCE_SIZE_AND_HASH],
            plaintext_size: PLAIN_PACKET_SIZE,
            pbuffer_prefix_size: PPACKET_PREFIX_SIZE,
            ciphertext_w_nonce_size: CIPTEXT_W_NONCE_SIZE,
            ciphertext_w_nonce_size_and_hmac: CIPTEXT_W_NONCE_SIZE_AND_HASH,
            writer,
            s_tools: None,
            seq_num_size: SEQ_NUM_SIZE,
            rng: OsRng::default(),
        }
    }

    /// # `write_str`
    ///
    /// Write a string to the socket being handled.
    /// If a cipher has been set this method encrypts the message befor
    /// sending it, plaintext and ciphertext have different sizes.
    pub async fn write_str(&mut self, msg: &str) -> Result<(), WriteHandlerError> {
        let msg = msg.as_bytes();
        self.prepare_packet(&msg).await?;

        match &self.s_tools {
            Some(s_tools) => {
                let nonce = Aes256Gcm::generate_nonce(&mut self.rng);
                let mut new_buff = s_tools
                    .aes_cipher
                    .encrypt(&nonce, &self.buffer[..])
                    .map_err(|e| WriteHandlerError::EncryptionError(anyhow::anyhow!("{}", e)))?;
                new_buff.extend(&nonce);
                for i in 0..new_buff.len() {
                    self.encrypte_buffer[i] = new_buff[i];
                }
                let mut hmac = <Hmac<Sha256> as Mac>::new_from_slice(&s_tools.hmac_key)?;
                hmac.update(&self.encrypte_buffer[..self.ciphertext_w_nonce_size]);
                let hash = hmac.finalize().into_bytes();
                let mut index = 0;
                for i in self.ciphertext_w_nonce_size..self.ciphertext_w_nonce_size_and_hmac {
                    self.encrypte_buffer[i] = hash[index];
                    index = index + 1;
                }
            }
            None => {}
        }

        self.send_packet().await?;

        Ok(())
    }

    /// `write_str`'s helper `send_packet`
    async fn send_packet(&mut self) -> Result<(), WriteHandlerError> {
        self.cursor = 0;
        match self.s_tools {
            Some(_) => {
                while self.cursor < self.ciphertext_w_nonce_size {
                    self.cursor = self.cursor
                        + self
                            .writer
                            .write(&self.encrypte_buffer[self.cursor..])
                            .await?;
                }
                while self.cursor < self.ciphertext_w_nonce_size_and_hmac {
                    self.cursor = self.cursor
                        + self
                            .writer
                            .write(&self.encrypte_buffer[self.cursor..])
                            .await?;
                }
            }
            None => {
                while self.cursor < self.plaintext_size {
                    self.cursor =
                        self.cursor + self.writer.write(&self.buffer[self.cursor..]).await?;
                }
            }
        }
        self.writer.flush().await?;
        Ok(())
    }

    /// `write_str`'s helper `prepare_packet`
    async fn prepare_packet(&mut self, msg: &[u8]) -> Result<usize, WriteHandlerError> {
        self.cursor = 0;
        let size = msg.len();
        if size > self.plaintext_size - self.pbuffer_prefix_size - self.seq_num_size {
            return Err(WriteHandlerError::MalformedPacket(anyhow::anyhow!(
                "Message to be sent is too long."
            )));
        }

        let size_bytes = Vec::from(size.to_string().as_bytes());
        let len_size_bytes = size_bytes.len();
        while self.cursor < (self.pbuffer_prefix_size - len_size_bytes) {
            self.buffer[self.cursor] = b'0';
            self.cursor = self.cursor + 1;
        }
        for &i in size_bytes.iter() {
            self.buffer[self.cursor] = i;
            self.cursor = self.cursor + 1;
        }
        match &mut self.s_tools {
            Some(s_tools) => {
                let seq_num_bytes = Vec::from(s_tools.seq_number.to_string().as_bytes());
                let len_seq_num_bytes = seq_num_bytes.len();
                while self.cursor
                    < self.pbuffer_prefix_size + (self.seq_num_size - len_seq_num_bytes)
                {
                    self.buffer[self.cursor] = b'0';
                    self.cursor = self.cursor + 1;
                }
                for &i in seq_num_bytes.iter() {
                    self.buffer[self.cursor] = i;
                    self.cursor = self.cursor + 1;
                }
                s_tools.seq_number = s_tools.seq_number.wrapping_add(1)
            }
            None => {}
        }
        for &i in msg.iter() {
            self.buffer[self.cursor] = i;
            self.cursor = self.cursor + 1;
        }
        while self.cursor < self.plaintext_size {
            self.buffer[self.cursor] = 0;
            self.cursor = self.cursor + 1;
        }

        Ok(self.cursor)
    }

    // TODO: comments, refactor
    // NOTE: no need to consider encryption here
    pub async fn write_bytes(&mut self, msg: &[u8]) -> Result<(), WriteHandlerError> {
        self.prepare_packet(&msg).await?;

        self.send_packet().await?;

        Ok(())
    }

    pub fn import_safety_tools(
        &mut self,
        aes_key: &GenericArray<
            u8,
            UInt<UInt<UInt<UInt<UInt<UInt<UTerm, B1>, B0>, B0>, B0>, B0>, B0>,
        >,
        hmac_key_bytes: &[u8],
        seq_number: u32,
    ) -> Result<(), anyhow::Error> {
        if hmac_key_bytes.len() != 32 {
            // This should not happen
            return Err(anyhow::anyhow!("Hmac Key bytes has the wrong size."));
        }
        let mut hmac_key = [0; 32];
        hmac_key.copy_from_slice(hmac_key_bytes);

        self.s_tools = Some(SafetyTools {
            aes_cipher: Aes256Gcm::new(aes_key),
            hmac_key,
            seq_number,
        });
        Ok(())
    }
}

#[derive(thiserror::Error)]
pub enum WriteHandlerError {
    #[error(transparent)]
    IoError(#[from] io::Error),
    #[error(transparent)]
    MalformedPacket(#[from] anyhow::Error),
    // TODO: something better
    #[error(transparent)]
    EncryptionError(anyhow::Error),
    #[error(transparent)]
    HmacError(#[from] InvalidLength),
}

impl Debug for WriteHandlerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        error_chain_fmt(self, f)
    }
}

struct SafetyTools {
    aes_cipher: AesGcm<Aes256, UInt<UInt<UInt<UInt<UTerm, B1>, B1>, B0>, B0>>,
    hmac_key: [u8; 32],
    seq_number: u32,
}
