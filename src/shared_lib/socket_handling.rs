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

/// # `RecvHandler`
///
/// This struct handles the receiving side of a socket
/// It takes care also of encryption and message integrity.
/// It acts differently based on the presence of a `cipher`
/// in its fields.
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
        let buffer_size = match &self.cipher {
            Some(_) => self.ciphertext_w_nonce_size,
            None => self.plaintext_size,
        };

        self.receive_packet(buffer_size).await?;

        self.decipher_packet().await?;

        let size = self.validate_packet()?;

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

        Ok(())
    }

    fn validate_packet(&mut self) -> Result<usize, RecvHandlerError> {
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
    async fn decipher_packet(&mut self) -> Result<(), RecvHandlerError> {
        // If cipher is set decifer the packet
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

/// # `WriteHandler`
///
/// This struct handles the writing side of a socket.
/// It takes care also of encryption and message integrity.
/// It acts differently based on the presence of a `cipher`
/// in its fields.
pub struct WriteHandler<T: AsyncWrite + Unpin + Send> {
    cursor: usize,
    buffer: [u8; PLAIN_PACKET_SIZE],
    encrypte_buffer: [u8; CIPTEXT_W_NONCE_SIZE],
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
            buffer: [0; PLAIN_PACKET_SIZE],
            encrypte_buffer: [0; CIPTEXT_W_NONCE_SIZE],
            plaintext_size: PLAIN_PACKET_SIZE,
            pbuffer_prefix_size: PPACKET_PREFIX_SIZE,
            ciphertext_w_nonce_size: CIPTEXT_W_NONCE_SIZE,
            writer,
            cipher: None,
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

        match &self.cipher {
            Some(c) => {
                let nonce = Aes256Gcm::generate_nonce(&mut self.rng);
                let mut new_buff = c
                    .encrypt(&nonce, &self.buffer[..])
                    .map_err(|e| WriteHandlerError::EncryptionError(anyhow::anyhow!("{}", e)))?;
                new_buff.extend(&nonce);
                for i in 0..new_buff.len() {
                    self.encrypte_buffer[i] = new_buff[i];
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
        match self.cipher {
            Some(_) => {
                while self.cursor < self.ciphertext_w_nonce_size {
                    self.cursor = self.cursor
                        + self
                            .writer
                            .write(&self.encrypte_buffer[self.cursor..])
                            .await?;
                    self.writer.flush().await?;
                }
            }
            None => {
                while self.cursor < self.plaintext_size {
                    self.cursor =
                        self.cursor + self.writer.write(&self.buffer[self.cursor..]).await?;
                    self.writer.flush().await?;
                }
            }
        }
        Ok(())
    }

    /// `write_str`'s helper `prepare_packet`
    async fn prepare_packet(&mut self, msg: &[u8]) -> Result<usize, WriteHandlerError> {
        self.cursor = 0;
        let size = msg.len();
        if size > self.plaintext_size - self.pbuffer_prefix_size {
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
