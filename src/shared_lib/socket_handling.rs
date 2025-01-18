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
    AeadCore, Aes256Gcm, AesGcm,
};
use anyhow::Context;
use rand::rngs::OsRng;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::auxiliaries::error_chain_fmt;

/// TODO: comments
pub const PLAIN_PACKET_SIZE: usize = 1024;
pub const PPACKET_PREFIX_SIZE: usize = 4;
pub const PLAIN_PAYLOAD_SIZE: usize = 1020;
pub const CIPTEXT_SIZE: usize = 1040;
pub const CIPTEXT_W_NONCE_SIZE: usize = 1052;
pub const NONCE_SIZE: usize = 12;

/// TODO: comment
pub struct RecvHandler<T: AsyncRead + Unpin + Send> {
    cursor: usize,
    buffer: [u8; CIPTEXT_W_NONCE_SIZE],
    plaintext_size: usize,
    pbuffer_prefix_size: usize,
    ciphertext_w_nonce_size: usize,
    ciphertext_size: usize,
    reader: T,
    cipher: Option<AesGcm<Aes256, UInt<UInt<UInt<UInt<UTerm, B1>, B1>, B0>, B0>>>,
}

impl<T: AsyncRead + Unpin + Send> RecvHandler<T> {
    pub fn new(reader: T) -> Self {
        Self {
            cursor: 0,
            buffer: [0; CIPTEXT_W_NONCE_SIZE],
            plaintext_size: PLAIN_PACKET_SIZE,
            pbuffer_prefix_size: PPACKET_PREFIX_SIZE,
            ciphertext_w_nonce_size: CIPTEXT_W_NONCE_SIZE,
            ciphertext_size: CIPTEXT_SIZE,
            reader,
            cipher: None,
        }
    }

    // TODO: comments, refactor
    // IDEA: we may want to define a new function,
    // keep buffer as a filed of RecvHandler, this way
    // we don't declare a new buffer every time
    // NOTE: if failed line will be cleared
    pub async fn recv_str(&mut self, line: &mut String) -> Result<usize, RecvHandlerError> {
        self.cursor = 0;
        line.clear();
        let buffer_size = match &self.cipher {
            Some(_) => self.ciphertext_w_nonce_size,
            None => self.plaintext_size,
        };
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
        match &self.cipher {
            Some(c) => {
                let nonce = &self.buffer[self.ciphertext_size..];
                let buffer = &c
                    .decrypt(nonce.into(), &self.buffer[..self.ciphertext_size])
                    .map_err(|e| RecvHandlerError::EncryptionError(anyhow::anyhow!(e)))?;
                for i in 0..self.plaintext_size {
                    self.buffer[i] = buffer[i];
                }
            }
            None => {}
        }

        // Validate packet
        let size: usize = str::from_utf8(&self.buffer[0..self.pbuffer_prefix_size])
            .context("Malformed size prefix")?
            .parse()
            .context("Malformed size prefix")?;
        if size > self.plaintext_size - self.pbuffer_prefix_size {
            return Err(RecvHandlerError::MalformedPacket(anyhow::anyhow!(
                "Size of the packet out of bound"
            )));
        }
        // TODO: we may make it more efficient
        let l =
            str::from_utf8(&self.buffer[self.pbuffer_prefix_size..self.pbuffer_prefix_size + size])
                .context("Line sent contained invalid UTF-8")?;
        *line = l.to_string();
        if size != line.len() {
            return Err(RecvHandlerError::MalformedPacket(anyhow::anyhow!(
                "Size prefix and lenght of the line are different."
            )));
        }

        Ok(self.cursor)
    }

    // TODO: comment
    // NOTE: no need to consider encryption here
    pub async fn recv_bytes(&mut self) -> Result<Vec<u8>, RecvHandlerError> {
        self.cursor = 0;
        // Receive packet
        while self.cursor < PLAIN_PACKET_SIZE {
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

        // Validate packet
        let size: usize = str::from_utf8(&self.buffer[0..self.pbuffer_prefix_size])
            .context("Malformed size prefix")?
            .parse()
            .context("Malformed size prefix")?;
        if size > PLAIN_PACKET_SIZE - self.pbuffer_prefix_size {
            return Err(RecvHandlerError::MalformedPacket(anyhow::anyhow!(
                "Size of the packet out of bound"
            )));
        }
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
    pub fn import_cipher(
        &mut self,
        cipher: AesGcm<Aes256, UInt<UInt<UInt<UInt<UTerm, B1>, B1>, B0>, B0>>,
    ) {
        self.cipher = Some(cipher)
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
}

impl Debug for RecvHandlerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        error_chain_fmt(self, f)
    }
}

// TODO: comment
pub struct WriteHandler<T: AsyncWrite + Unpin + Send> {
    cursor: usize,
    plaintext_size: usize,
    pbuffer_prefix_size: usize,
    ciphertext_w_nonce_size: usize,
    writer: T,
    cipher: Option<AesGcm<Aes256, UInt<UInt<UInt<UInt<UTerm, B1>, B1>, B0>, B0>>>,
    rng: OsRng,
}

impl<T: AsyncWrite + Unpin + Send> WriteHandler<T> {
    pub fn new(writer: T) -> Self {
        Self {
            cursor: 0,
            plaintext_size: PLAIN_PACKET_SIZE,
            pbuffer_prefix_size: PPACKET_PREFIX_SIZE,
            ciphertext_w_nonce_size: CIPTEXT_W_NONCE_SIZE,
            writer,
            cipher: None,
            rng: OsRng::default(),
        }
    }
    /// TODO: comment, refactor
    pub async fn write_str(&mut self, msg: &str) -> Result<(), WriteHandlerError> {
        let mut size = msg.len();
        if size > self.plaintext_size - self.pbuffer_prefix_size {
            return Err(WriteHandlerError::MalformedPacket(anyhow::anyhow!(
                "Message to be sent is too long."
            )));
        }
        let mut pack = size.to_string();
        while pack.len() < self.pbuffer_prefix_size {
            pack.insert(0, '0');
        }
        pack.push_str(msg);
        size = pack.len();

        let mut diff = self.plaintext_size - size;
        while diff > 0 {
            pack.push('0');
            size = pack.len();
            diff = self.plaintext_size - size;
        }
        let buffer = pack.as_bytes();

        // TODO: solve code duplication
        self.cursor = 0;
        match &self.cipher {
            Some(c) => {
                let nonce = Aes256Gcm::generate_nonce(&mut self.rng);
                let mut new_buff = c
                    .encrypt(&nonce, buffer)
                    .map_err(|e| WriteHandlerError::EncryptionError(anyhow::anyhow!("{}", e)))?;
                new_buff.extend(&nonce);

                while self.cursor < self.ciphertext_w_nonce_size {
                    self.cursor = self.cursor + self.writer.write(&new_buff[self.cursor..]).await?;
                    self.writer.flush().await?;
                }
            }
            None => {
                while self.cursor < self.plaintext_size {
                    self.cursor = self.cursor + self.writer.write(&buffer[self.cursor..]).await?;
                    self.writer.flush().await?;
                }
            }
        }

        Ok(())
    }
    // TODO: comments, refactor
    // NOTE: no need to consider encryption here
    pub async fn write_bytes(&mut self, msg: &[u8]) -> Result<(), WriteHandlerError> {
        let mut size = msg.len();
        if size > self.plaintext_size - self.pbuffer_prefix_size {
            return Err(WriteHandlerError::MalformedPacket(anyhow::anyhow!(
                "Message to be sent is too long."
            )));
        }
        let mut pack = Vec::from(size.to_string().as_bytes());
        while pack.len() < self.pbuffer_prefix_size {
            pack.insert(0, b'0');
        }
        pack.extend_from_slice(msg);
        size = pack.len();

        let mut diff = self.plaintext_size - size;
        while diff > 0 {
            pack.push(b'0');
            size = pack.len();
            diff = self.plaintext_size - size;
        }

        self.cursor = 0;
        while self.cursor < size {
            self.cursor = self.cursor + self.writer.write(&pack[self.cursor..]).await?;
            self.writer.flush().await?;
        }

        Ok(())
    }
    pub fn import_cipher(
        &mut self,
        cipher: AesGcm<Aes256, UInt<UInt<UInt<UInt<UTerm, B1>, B1>, B0>, B0>>,
    ) {
        self.cipher = Some(cipher)
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
}

impl Debug for WriteHandlerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        error_chain_fmt(self, f)
    }
}
