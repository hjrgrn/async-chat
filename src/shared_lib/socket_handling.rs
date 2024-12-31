use core::str;
use std::fmt::Debug;
use std::usize;

use anyhow::Context;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const BUFF_SIZE: usize = 128;

pub struct RecvHandler {
    cursor: usize,
    buffer: [u8; BUFF_SIZE],
    size_prefix_len: usize,
}

impl RecvHandler {
    pub fn new() -> Self {
        Self {
            cursor: 0,
            buffer: [0; BUFF_SIZE],
            size_prefix_len: BUFF_SIZE.to_string().len(),
        }
    }

    // TODO: comments
    // IDEA: we may want to define a new function,
    // keep buffer as a filed of RecvHandler, this way
    // we don't declare a new buffer every time
    // NOTE: if failed line will be cleared
    pub async fn recv<T: AsyncRead + Unpin + Send>(
        &mut self,
        line: &mut String,
        reader: &mut T,
    ) -> Result<usize, RecvHandlerError> {
        self.cursor = 0;
        line.clear();
        // Receive packet
        while self.cursor < BUFF_SIZE {
            match reader.read(&mut self.buffer[self.cursor..]).await {
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
        let size: usize = str::from_utf8(&self.buffer[0..self.size_prefix_len])
            .context("Malformed size prefix")?
            .parse()
            .context("Malformed size prefix")?;
        if size > BUFF_SIZE - self.size_prefix_len {
            return Err(RecvHandlerError::MalformedPacket(anyhow::anyhow!(
                "Size of the packet out of bound"
            )));
        }
        // TODO: we may make it more efficient
        let l = str::from_utf8(&self.buffer[3..self.size_prefix_len + size])
            .context("Line sent contained invalid UTF-8")?;
        *line = l.to_string();
        if size != line.len() {
            return Err(RecvHandlerError::MalformedPacket(anyhow::anyhow!(
                "Size prefix and lenght of the line are different."
            )));
        }

        Ok(self.cursor)
    }
}

// TODO: custom Debug showing source
#[derive(thiserror::Error, Debug)]
pub enum RecvHandlerError {
    #[error("Peer closed the connection.")]
    ConnectionInterrupted,
    #[error(transparent)]
    IoError(#[from] io::Error),
    #[error(transparent)]
    MalformedPacket(#[from] anyhow::Error),
}

pub struct WriteHandler {
    cursor: usize,
    buffer_size: usize,
    size_prefix_len: usize,
}

impl WriteHandler {
    pub fn new() -> Self {
        Self {
            cursor: 0,
            buffer_size: BUFF_SIZE,
            size_prefix_len: BUFF_SIZE.to_string().len(),
        }
    }
    pub async fn write<T: AsyncWrite + Unpin + Send>(
        &mut self,
        msg: &str,
        writer: &mut T,
    ) -> Result<(), WriteHandlerError> {
        let mut size = msg.len();
        if size > self.buffer_size - self.size_prefix_len {
            return Err(WriteHandlerError::MalformedPacket(anyhow::anyhow!(
                "Message to be sent is too long."
            )));
        }
        let mut pack = size.to_string();
        while pack.len() < self.size_prefix_len {
            pack.insert(0, '0');
        }
        pack.push_str(msg);
        size = pack.len();

        let mut diff = self.buffer_size - size;
        while diff > 0 {
            pack.push('0');
            size = pack.len();
            diff = self.buffer_size - size;
        }
        let buffer = pack.as_bytes();

        self.cursor = 0;
        while self.cursor < size {
            self.cursor = self.cursor + writer.write(&buffer[self.cursor..]).await?;
            writer.flush().await?;
        }

        Ok(())
    }
}

// TODO: custom Debug showing source
#[derive(thiserror::Error, Debug)]
pub enum WriteHandlerError {
    #[error(transparent)]
    IoError(#[from] io::Error),
    #[error(transparent)]
    MalformedPacket(#[from] anyhow::Error),
}
